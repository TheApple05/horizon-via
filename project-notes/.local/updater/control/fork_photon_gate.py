"""Run the existing Photon acceptance harness with the candidate's exact binaries."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from lab import game_env


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--photon", type=Path, required=True)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--resources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profile", choices=("high", "ultra"), default="high")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    stub = output / "candidate-control/target/release"
    stub.mkdir(parents=True)
    binaries = args.binaries.resolve()
    shutil.copy2(binaries / "horizon-ctl.exe", stub / "horizon-ctl.exe")
    game = binaries / "horizon-client.exe"
    command = [sys.executable, str(args.photon.resolve() / "tools/verify_runtime.py"),
               str(args.photon.resolve() / "dist/Photon-Original-40c8871.zip"),
               "--horizon", str(stub.parent.parent), "--game", str(game),
               "--resources", str(args.resources.resolve()), "--compat", "--profile", args.profile,
               "--output", str(output / "captures")]
    env = {k: v for k, v in os.environ.items() if not k.startswith("HORIZON_")}
    env = game_env(env)
    env["HORIZON_TELEMETRY_URL"] = "off"
    result = subprocess.run(command, env=env, timeout=1800,
                            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    def sha(path):
        with path.open("rb") as source:
            return hashlib.file_digest(source, "sha256").hexdigest()
    reports = list((output / "captures").glob("runtime-*/report.json"))
    passed = result.returncode == 0 and len(reports) == 1 and json.loads(reports[0].read_text())["ok"]
    proof = {"passed": passed, "profile": args.profile, "game_sha256": sha(game),
             "ctl_sha256": sha(binaries / "horizon-ctl.exe"), "reports": [str(p) for p in reports]}
    (output / "result.json").write_text(json.dumps(proof, indent=2) + "\n")
    print(json.dumps(proof))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
