"""Persistent trusted supervisor: poll upstream while one isolated worker tests.

Run with pythonw on Windows to keep the service window hidden. Stopping this
service terminates its worker process tree; a later start recovers its queue row.
"""
from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import time

from fork_updater import Updater, lock

LOG_LIMIT = 2 * 1024 * 1024


class Supervisor:
    def __init__(self, config):
        self.updater = Updater(config)
        self.state = self.updater.state
        self.control = Path(__file__).resolve().with_name("fork_updater.py")
        self.worker = None
        self.worker_log = None
        self.snapshot = {"pid": os.getpid(), "started": time.time(), "state": "starting"}

    def record(self, event, **details):
        self.snapshot.update(updated=time.time(), state=event, **details)
        temporary = self.state / "service-status.json.tmp"
        temporary.write_text(json.dumps(self.snapshot, indent=2) + "\n", encoding="utf-8")
        os.replace(temporary, self.state / "service-status.json")
        log = self.state / "service.log"
        self.rotate(log)
        with log.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(dict(time=time.time(), event=event, **details)) + "\n")

    @staticmethod
    def rotate(path):
        if path.exists() and path.stat().st_size >= LOG_LIMIT:
            os.replace(path, path.with_suffix(path.suffix + ".1"))

    def poll(self):
        try:
            result = self.updater.poll()
            self.record("polled", last_poll=time.time(), poll_error=None, **result)
            return True
        except Exception as error:
            # Status deliberately stores no raw exception, environment or command output.
            self.record("poll_failed", last_poll=time.time(), poll_error=type(error).__name__)
            return False

    def start_worker(self):
        self.updater.gates()
        log_path = self.state / "worker.log"
        self.rotate(log_path)
        self.worker_log = log_path.open("ab")
        try:
            self.worker = subprocess.Popen(
                [sys.executable, str(self.control), "--config", str(self.updater.config_path), "run"],
                stdin=subprocess.DEVNULL, stdout=self.worker_log, stderr=subprocess.STDOUT,
                creationflags=(subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP)
                if os.name == "nt" else 0, start_new_session=os.name != "nt")
        except Exception:
            self.worker_log.close()
            self.worker_log = None
            raise
        self.record("testing", worker_pid=self.worker.pid)

    def supervise(self, interval, once=False):
        with lock(self.state / "supervisor.lock"):
            if once:
                if not self.poll():
                    return 1
                result = self.updater.run()
                self.record("once_complete", result_status=result["status"],
                            upstream=result.get("upstream"), candidate=result.get("candidate"))
                return 1 if result["status"] == "failed" else 0
            next_poll = 0.0
            next_worker = 0.0
            try:
                while True:
                    if time.monotonic() >= next_poll:
                        self.poll()
                        next_poll = time.monotonic() + interval
                    if self.worker is not None and self.worker.poll() is not None:
                        code = self.worker.returncode
                        self.worker_log.close()
                        self.worker_log = self.worker = None
                        self.record("worker_finished", worker_pid=None, worker_exit_code=code)
                    if self.worker is None and time.monotonic() >= next_worker:
                        counts = self.updater.status(limit=1)["counts"]
                        if counts.get("queued", 0) or counts.get("running", 0):
                            try:
                                backpressure = self.updater.disk_backpressure()
                                if backpressure:
                                    self.record("disk_backpressure", **{
                                        key: value for key, value in backpressure.items() if key != "status"})
                                    next_worker = time.monotonic() + interval
                                else:
                                    self.start_worker()
                            except Exception as error:
                                self.record("worker_start_failed", worker_error=type(error).__name__)
                                next_worker = time.monotonic() + interval
                    time.sleep(min(2.0, max(0.1, next_poll - time.monotonic())))
            finally:
                if self.worker is not None and self.worker.poll() is None:
                    Updater._terminate(self.worker)
                if self.worker_log is not None:
                    self.worker_log.close()
                self.record("stopped", worker_pid=None)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True)
    parser.add_argument("--interval", type=float, default=120)
    parser.add_argument("--once", action="store_true")
    args = parser.parse_args()
    if not math.isfinite(args.interval) or args.interval < 1:
        parser.error("interval must be finite and at least one second")
    supervisor = Supervisor(args.config)
    try:
        return supervisor.supervise(args.interval, args.once)
    except KeyboardInterrupt:
        return 130
    finally:
        supervisor.updater.db.close()


if __name__ == "__main__":
    raise SystemExit(main())
