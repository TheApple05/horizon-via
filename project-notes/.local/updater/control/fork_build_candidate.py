"""Build and freeze exact candidate binaries before runtime gates can use them."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
from lab import game_env


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    def git(*arguments):
        return subprocess.check_output(["git", "-C", str(args.candidate), *arguments], text=True).strip()
    revision = git("rev-parse", "HEAD")
    if git("status", "--porcelain", "--untracked-files=no"):
        raise RuntimeError("Commit the candidate before building frozen binaries")
    subprocess.run(["cargo", "build", "--locked", "--profile", "fast", "-p", "horizon-client",
                    "-p", "horizon-client-lib", "--bins"], cwd=args.candidate, check=True, env=game_env())
    if git("rev-parse", "HEAD") != revision or git("status", "--porcelain", "--untracked-files=no"):
        raise RuntimeError("Candidate changed during build; refusing to label these binaries")
    target = Path(os.environ.get("CARGO_TARGET_DIR", args.candidate / "target"))
    args.output.mkdir(parents=True, exist_ok=False)
    suffix = ".exe" if os.name == "nt" else ""
    proof = {}
    for name in ("horizon-client", "horizon-ctl"):
        dest = args.output / (name + suffix)
        shutil.copy2(target / "fast" / dest.name, dest)
        with dest.open("rb") as source:
            proof[dest.name] = hashlib.file_digest(source, "sha256").hexdigest()
    proof["candidate"] = revision
    (args.output / "proof.json").write_text(json.dumps(proof, indent=2) + "\n")
    print(json.dumps(proof))


if __name__ == "__main__":
    main()
