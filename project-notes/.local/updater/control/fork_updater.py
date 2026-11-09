"""Trusted local control plane for isolated, durable upstream compatibility checks.

Load configuration from outside candidate trees. No publishing is performed here.
Failed worktrees and all logs are retained. Successful candidates remain as Git refs.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import time
import uuid

from lab import game_env

REQUIRED = {"build", "regression", "runtime", "mods"}


@contextmanager
def lock(path):
    """OS releases the lock after process death; stale files are harmless."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+b") as stream:
        if os.fstat(stream.fileno()).st_size == 0:
            stream.write(b"0")
            stream.flush()
        stream.seek(0)
        try:
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            raise RuntimeError(f"Another process holds {path.name}") from error
        try:
            yield
        finally:
            stream.seek(0)
            if os.name == "nt":
                msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(stream, fcntl.LOCK_UN)


class Updater:
    def __init__(self, config_path):
        self.config_path = Path(config_path).resolve()
        raw = self.config_path.read_bytes()
        self.config = json.loads(raw)
        self.config_hash = hashlib.sha256(raw).hexdigest()
        self.repo = Path(self.config["repo"]).resolve()
        self.state = Path(self.config["state_dir"]).resolve()
        self.state.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(self.state / "queue.sqlite3", timeout=30)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA synchronous=FULL")
        self.db.executescript("""
            CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS queue (
                id INTEGER PRIMARY KEY, upstream TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL DEFAULT 'queued', base TEXT, candidate TEXT,
                run_dir TEXT, started REAL, finished REAL, error TEXT,
                gates TEXT, config_hash TEXT);
        """)
        self.db.commit()

    def git(self, *args, cwd=None):
        result = subprocess.run(["git", "-C", str(cwd or self.repo), *args],
                                capture_output=True, text=True, timeout=300,
                                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        if result.returncode:
            raise RuntimeError(f"git {args[0]} failed: {result.stderr.strip()[:2000]}")
        return result.stdout.strip()

    def meta(self, key):
        row = self.db.execute("SELECT value FROM meta WHERE key=?", (key,)).fetchone()
        return row[0] if row else None

    def init(self, baseline):
        baseline = self.git("rev-parse", "--verify", baseline + "^{commit}")
        with lock(self.state / "poll.lock"), self.db:
            if self.meta("cursor"):
                raise RuntimeError("Already initialized; refusing to discard queue history")
            self.db.execute("INSERT INTO meta VALUES ('cursor', ?)", (baseline,))
        return {"baseline": baseline, "note": "Baseline itself is not marked validated"}

    def poll(self):
        with lock(self.state / "poll.lock"):
            cursor = self.meta("cursor")
            if not cursor:
                raise RuntimeError("Run init with an explicitly reviewed baseline first")
            for remote in ("upstream", "origin"):
                self.git("fetch", "--no-tags", remote,
                         f"+refs/heads/main:refs/remotes/{remote}/main")
            head = self.git("rev-parse", "upstream/main")
            # Refuse a rewrite before changing either queue or watermark.
            self.git("merge-base", "--is-ancestor", cursor, head)
            commits = self.git("rev-list", "--reverse", "--topo-order", f"{cursor}..{head}").splitlines()
            with self.db:
                self.db.executemany("INSERT OR IGNORE INTO queue(upstream) VALUES (?)",
                                    ((sha,) for sha in commits))
                self.db.execute("UPDATE meta SET value=? WHERE key='cursor'", (head,))
            return {"upstream": head, "discovered": len(commits)}

    def gates(self):
        gates = self.config.get("gates", [])
        if not REQUIRED <= {gate.get("category") for gate in gates}:
            raise ValueError("Configuration needs build, regression, runtime and mods gates")
        names = set()
        for gate in gates:
            name = gate.get("name", "")
            argv = gate.get("argv")
            if not re.fullmatch(r"[A-Za-z0-9_-]+", name) or name in names:
                raise ValueError("Gate names must be unique safe filenames")
            if not isinstance(argv, list) or not argv or not all(isinstance(a, str) for a in argv):
                raise ValueError("Gate argv must be a nonempty string array")
            if not 0 < gate.get("timeout", 3600) <= 86400:
                raise ValueError("Gate timeout must be 1..86400 seconds")
            if "success_regex" in gate:
                re.compile(gate["success_regex"])
            names.add(name)
        return gates

    def run(self):
        gates = self.gates()  # Invalid configuration never claims a queue item.
        with lock(self.state / "worker.lock"):
            with self.db:
                self.db.execute("UPDATE queue SET status='queued', error='Interrupted; retrying' "
                                "WHERE status='running'")
                row = self.db.execute("SELECT * FROM queue WHERE status='queued' ORDER BY id LIMIT 1").fetchone()
                if row is None:
                    return {"status": "idle"}
                backpressure = self.disk_backpressure()
                if backpressure:
                    return backpressure
                self.db.execute("UPDATE queue SET status='running', started=?, finished=NULL, "
                                "error=NULL, gates=NULL, config_hash=? WHERE id=?",
                                (time.time(), self.config_hash, row["id"]))
            return self._candidate(row, gates)

    def disk_backpressure(self):
        minimum = float(self.config.get("min_free_gb", 20))
        if not 0 <= minimum < float("inf"):
            raise ValueError("min_free_gb must be finite and nonnegative")
        free = shutil.disk_usage(self.state).free
        if free < minimum * 1024 ** 3:
            return {"status": "backpressure", "reason": "disk", "free_bytes": free,
                    "required_bytes": int(minimum * 1024 ** 3)}
        return None

    def _candidate(self, row, gates):
        run_dir = self.state / "runs" / f"{row['id']}-{uuid.uuid4().hex[:12]}"
        candidate = run_dir / "source"
        run_dir.mkdir(parents=True)
        results = []
        base = commit = None
        try:
            if self.config_path.is_relative_to(candidate):
                raise ValueError("Trusted config must be outside candidate")
            base = self.git("rev-parse", "origin/main")
            self.git("worktree", "add", "--detach", str(candidate), base)
            message = (f"Merge upstream {row['upstream']} for compatibility validation\n\n"
                       "Co-Authored-By: Codex <noreply@openai.com>")
            self.git("-c", "core.hooksPath=", "-c", "user.name=Horizon Updater",
                     "-c", "user.email=noreply@openai.com", "merge", "--no-edit", "--no-verify",
                     "-m", message, row["upstream"], cwd=candidate)
            commit = self.git("rev-parse", "HEAD", cwd=candidate)
            with self.db:
                self.db.execute("UPDATE queue SET base=?, candidate=?, run_dir=? WHERE id=?",
                                (base, commit, str(run_dir), row["id"]))
            values = dict(candidate=str(candidate), upstream=row["upstream"],
                          run_dir=str(run_dir), python=sys.executable)
            env = os.environ.copy()
            env.update({k: str(v).format(**values) for k, v in self.config.get("env", {}).items()})
            for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
                        "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
                home = run_dir / key.lower()
                home.mkdir()
                env[key] = str(home)
            env = game_env(env)
            for key in ("HORIZON_SESSION_FILE", "HORIZON_SESSION_STDIN", "HORIZON_PLAYER_NAME"):
                env.pop(key, None)
            for gate in gates:
                argv = [arg.format(**values) for arg in gate["argv"]]
                cwd = gate.get("cwd", "{candidate}").format(**values)
                log_path = run_dir / (gate["name"] + ".log")
                with log_path.open("wb") as log:
                    process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT,
                                               creationflags=(subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW)
                                               if os.name == "nt" else 0,
                                               start_new_session=os.name != "nt")
                    try:
                        code = process.wait(timeout=gate.get("timeout", 3600))
                    except subprocess.TimeoutExpired:
                        self._terminate(process)
                        raise RuntimeError(f"Gate {gate['name']} timed out")
                passed = code == 0
                if passed and "success_regex" in gate:
                    passed = re.search(gate["success_regex"], log_path.read_text(errors="replace")) is not None
                results.append(dict(name=gate["name"], category=gate["category"], passed=passed,
                                    exit_code=code, log=str(log_path)))
                with self.db:
                    self.db.execute("UPDATE queue SET gates=? WHERE id=?", (json.dumps(results), row["id"]))
                if not passed:
                    raise RuntimeError(f"Gate {gate['name']} failed; inspect its private log")
            if self.git("rev-parse", "HEAD", cwd=candidate) != commit or self.git(
                    "status", "--porcelain", "--untracked-files=no", cwd=candidate):
                raise RuntimeError("Tests changed candidate source or HEAD; validation refused")
            self.git("update-ref", f"refs/fork-updater/validated/{row['upstream']}", commit)
            resolved = candidate.resolve(strict=True)
            if candidate.is_symlink() or not resolved.is_relative_to((self.state / "runs").resolve()):
                raise RuntimeError("Successful worktree cleanup path escaped state/runs")
            self.git("worktree", "remove", str(resolved))
            status, error = "validated", None
        except Exception as failure:
            status, error = "failed", str(failure)[:3000]
        with self.db:
            self.db.execute("UPDATE queue SET status=?, base=?, candidate=?, run_dir=?, finished=?, "
                            "error=?, gates=? WHERE id=?", (status, base, commit, str(run_dir), time.time(),
                                                          error, json.dumps(results), row["id"]))
        return {"upstream": row["upstream"], "status": status, "error": error,
                "candidate": commit, "run_dir": str(run_dir), "merge_ready": status == "validated"}

    @staticmethod
    def _terminate(process):
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True,
                           creationflags=subprocess.CREATE_NO_WINDOW)
        else:
            import signal
            os.killpg(process.pid, signal.SIGKILL)
        process.wait()

    def status(self, limit=10):
        counts = dict(self.db.execute("SELECT status, count(*) FROM queue GROUP BY status"))
        rows = [dict(row) for row in self.db.execute("SELECT * FROM queue ORDER BY id DESC LIMIT ?", (limit,))]
        for row in rows:
            row["gates"] = json.loads(row["gates"] or "[]")
            row["merge_ready"] = row["status"] == "validated"
        return {"cursor": self.meta("cursor"), "counts": counts, "recent": rows}

    def retry(self, upstream):
        with lock(self.state / "worker.lock"), self.db:
            row = self.db.execute("SELECT * FROM queue WHERE upstream=? AND status IN ('failed', 'validated')",
                                  (upstream,)).fetchone()
            if row is None:
                raise ValueError("Retry requires the full SHA of an existing failed or validated item")
            if row["status"] == "validated" and row["candidate"]:
                self.git("update-ref", f"refs/fork-updater/attempts/{upstream}/{row['candidate']}", row["candidate"])
            changed = self.db.execute("UPDATE queue SET status='queued', error=NULL "
                                      "WHERE upstream=? AND status IN ('failed', 'validated')", (upstream,)).rowcount
            if changed != 1:
                raise RuntimeError("Queue item changed before retry could be recorded")
        return {"upstream": upstream, "status": "queued", "previous_attempts_retained": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("init").add_argument("--baseline", required=True)
    commands.add_parser("poll")
    commands.add_parser("run")
    commands.add_parser("retry", help="Requeue a failed or validated item against the current fork base").add_argument(
        "--upstream", required=True, help="Full upstream SHA of the failed or validated queue item")
    commands.add_parser("status").add_argument("--limit", type=int, default=10)
    args = parser.parse_args()
    updater = Updater(args.config)
    try:
        if args.command == "init":
            result = updater.init(args.baseline)
        elif args.command == "status":
            result = updater.status(max(1, min(args.limit, 100)))
        elif args.command == "retry":
            result = updater.retry(args.upstream)
        else:
            result = getattr(updater, args.command)()
        print(json.dumps(result, indent=2))
        return 1 if result.get("status") == "failed" else 0
    except Exception as error:
        print(json.dumps({"error": str(error)}))
        return 1
    finally:
        updater.db.close()


if __name__ == "__main__":
    raise SystemExit(main())
