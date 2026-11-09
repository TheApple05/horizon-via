"""Test the prepared TerraForged native mod against an exact candidate commit.

Retargets copies of the reviewed stage/probe helpers and native bundle only.
The original distribution and its compatibility pin are never modified.
This gate checks prepared terrain reconstruction, not procedural worldgen or UI.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command: list[str], *, log: Path, env: dict[str, str], timeout: int = 1800) -> None:
    with log.open("wb") as stream:
        result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT,
                                env=env, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"{Path(command[0]).name} failed ({result.returncode}); see {log}")


def git(candidate: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(candidate), *args], text=True).strip()


TERRA_SCOPE = ("horizon/horizon-native.json", "horizon/tools/stage.py",
               "horizon/tools/run_probe.py", "horizon/tests/prepared_probe.rs",
               "horizon/mixins/terraforged_prepared")


def check_terra_revision(terra: Path, expected: str) -> str:
    if not re.fullmatch(r"[0-9a-f]{40}", expected):
        raise ValueError("Invalid configured TerraForged commit")
    if Path(git(terra, "rev-parse", "--show-toplevel")).resolve() != terra.resolve():
        raise ValueError("--terra-root must name the TerraForged Git worktree root")
    actual = git(terra, "rev-parse", "HEAD")
    if actual != expected:
        raise ValueError("TerraForged HEAD changed; review the new revision and rerun all gates")
    if git(terra, "status", "--porcelain", "--untracked-files=all", "--", *TERRA_SCOPE):
        raise ValueError("TerraForged gate inputs have uncommitted changes")
    return actual


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("candidate", "terra-root", "mixins", "resources", "snapshot", "biomes", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--expected-mod-sha256", required=True)
    parser.add_argument("--expected-terra-commit",
                        help="Reviewed commit in the dedicated TerraForged-Horizon repository")
    args = parser.parse_args()
    output = args.output.resolve()
    report: dict = {"schema": 1, "status": "running", "runtimeExecuted": False,
                    "startedAt": dt.datetime.now(dt.timezone.utc).isoformat()}
    originals: dict[Path, str] = {}
    created_output = False
    try:
        candidate, terra = args.candidate.resolve(strict=True), args.terra_root.resolve(strict=True)
        if args.expected_terra_commit:
            report["terraRepositoryCommit"] = check_terra_revision(terra, args.expected_terra_commit)
        if Path(git(candidate, "rev-parse", "--show-toplevel")).resolve() != candidate:
            raise ValueError("--candidate must name the Git worktree root")
        if output.exists() or output.is_relative_to(terra):
            raise ValueError("Choose a new output directory outside the original TerraForged project")
        if not re.fullmatch(r"[0-9a-f]{64}", args.expected_mod_sha256):
            raise ValueError("--expected-mod-sha256 must be the frozen production Java mod SHA-256")
        head = git(candidate, "rev-parse", "HEAD")
        # Only these exact committed crates are exported by the reviewed stage helper.
        scope = ["crates/horizon-core", "crates/horizon-world", "Cargo.lock"]
        if git(candidate, "diff", "--name-only", "HEAD", "--", *scope):
            raise ValueError("Candidate core/world/lock have uncommitted changes; commit before testing")
        metadata_path = terra / "horizon/horizon-native.json"
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
        old_pin = metadata["horizon"]
        if not re.fullmatch(r"[0-9a-f]{40}", old_pin):
            raise ValueError("Invalid original native bundle source pin")
        required = [metadata_path, terra / "horizon/tools/stage.py", terra / "horizon/tools/run_probe.py",
                    terra / "horizon/tests/prepared_probe.rs"]
        bundle = terra / "horizon/mixins/terraforged_prepared"
        required += sorted(path for path in bundle.rglob("*") if path.is_file())
        for path in required:
            if path.is_symlink() or not path.resolve().is_relative_to(terra):
                raise ValueError("Original native bundle contains a link outside its project")
            originals[path] = sha(path)
        snapshot = args.snapshot.resolve(strict=True)
        manifest_path = snapshot / "terraforged-prepared.json"
        corpus = json.loads(manifest_path.read_text(encoding="utf-8"))
        if corpus.get("sourceModSha256") != args.expected_mod_sha256 or not corpus.get("chunks"):
            raise ValueError("Snapshot does not match the expected production Java mod or is empty")
        resources = args.resources.resolve(strict=True)
        biomes = args.biomes.resolve(strict=True)
        if not list(biomes.glob("*.json")):
            raise ValueError("Prepared biome definitions are missing")
        if not (resources / "horizon-resources.json").is_file():
            raise ValueError("Owned prepared Horizon resources are missing")
        mixins = args.mixins.resolve(strict=True)
        output.mkdir(parents=True)
        created_output = True
        project = output / "isolated-terra"
        for path in required:
            destination = project / path.relative_to(terra)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, destination)
            if sha(destination) != originals[path]:
                raise ValueError("TerraForged input changed while copying")
        if args.expected_terra_commit:
            check_terra_revision(terra, args.expected_terra_commit)
        for relative in ("horizon/tools/stage.py", "horizon/tools/run_probe.py",
                         "horizon/mixins/terraforged_prepared/horizon-mixin.toml", "horizon/horizon-native.json"):
            path = project / relative
            text = path.read_text(encoding="utf-8")
            if old_pin not in text:
                raise ValueError(f"Reviewed source pin contract changed: {relative}")
            path.write_text(text.replace(old_pin, head), encoding="utf-8", newline="\n")
        # Retain the original oracle and all malformed-input checks. Only extend the build
        # timeout and isolate every launcher path; neither change weakens a verdict.
        probe_helper = project / "horizon/tools/run_probe.py"
        text = probe_helper.read_text(encoding="utf-8").replace("timeout=180)", "timeout=1800)", 1)
        text = text.replace('"HORIZON_NO_AUDIO": "1", "HORIZON_LAUNCHER_NO_SELF_UPDATE": "1"',
                            '"HORIZON_NO_AUDIO": "1", "HORIZON_LAUNCHER_NO_SELF_UPDATE": "1", '
                            '"HORIZON_LAUNCHER_TRASH": str(work / "trash"), '
                            '"HORIZON_LAUNCHER_CLIPBOARD": str(work / "clipboard"), '
                            '"HORIZON_LAUNCHER_DESKTOP": str(work / "desktop")')
        probe_helper.write_text(text, encoding="utf-8", newline="\n")
        ported = {str(path.relative_to(project)).replace("\\", "/"): sha(path)
                  for path in project.rglob("*") if path.is_file()}
        report.update({"candidateCommit": head, "originalPackageHorizonPin": old_pin,
                       "portedTestRevision": head, "originalPackageMixinsPin": metadata["mixins"],
                       "mixinsExecutableSha256": sha(mixins), "snapshotManifestSha256": sha(manifest_path),
                       "resourcesManifestSha256": sha(resources / "horizon-resources.json"),
                       "biomeDefinitionSha256": {path.name: sha(path) for path in sorted(biomes.glob("*.json"))},
                       "javaModSha256": args.expected_mod_sha256,
                       "originalSourceSha256": {path.relative_to(terra).as_posix(): value
                                                for path, value in originals.items()},
                       "portedSourceSha256": ported,
                       "scope": "Exact committed core/world crates, native constructor injection and Java prepared chunk oracle; no procedural or rendered-client claim"})
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        environment = os.environ.copy()
        # The reviewed helper expects its executable under the staged workspace target.
        for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_TARGET",
                    "HORIZON_SESSION_FILE", "HORIZON_PLAYER_NAME"):
            environment.pop(key, None)
        stage_output = project / "build/qa/horizon/current-candidate"
        run([sys.executable, str(project / "horizon/tools/stage.py"), "--horizon", str(candidate),
             "--mixins", str(mixins), "--output", str(stage_output)], log=output / "stage.log", env=environment)
        run([sys.executable, str(probe_helper), "--stage", str(stage_output / "staged"),
             "--resources", str(resources), "--snapshot", str(snapshot), "--biomes", str(biomes),
             "--expected-mod-sha256", args.expected_mod_sha256, "--report", str(output / "probe.json")],
            log=output / "probe.log", env=environment)
        verdict = json.loads((output / "probe.json").read_text(encoding="utf-8"))
        if (verdict.get("status") != "passed-prepared-terrain-native-probe"
                or verdict.get("horizonRevision") != head
                or not verdict.get("allJavaBlockAndQuartBiomeDigestsMatch")
                or not verdict.get("expectedDecoderFailures")):
            raise ValueError("Probe did not establish candidate-bound native compatibility")
        if git(candidate, "rev-parse", "HEAD") != head:
            raise ValueError("Candidate HEAD changed during gate")
        if args.expected_terra_commit:
            check_terra_revision(terra, args.expected_terra_commit)
        report.update({"status": "passed", "runtimeExecuted": True,
                       "probeReport": str(output / "probe.json"), "blockCount": verdict["blockCount"],
                       "quartBiomeCount": verdict["quartBiomeCount"], "chunkCount": verdict["chunkCount"]})
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as error:
        report.update(status="failed", error=str(error))
    finally:
        unchanged = all(path.is_file() and sha(path) == value for path, value in originals.items())
        report["originalPackageUnchanged"] = unchanged
        if not unchanged:
            report.update(status="failed", error="Original native source changed during gate")
        report["finishedAt"] = dt.datetime.now(dt.timezone.utc).isoformat()
        if created_output:
            (output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, indent=2))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
