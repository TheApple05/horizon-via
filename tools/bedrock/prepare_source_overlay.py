#!/usr/bin/env python3
"""Prepare exact-d8 developer source-overlay inputs; never install or execute them."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import unicodedata

BASE = "d8edfbcd0f70aac5b41bab12b8bbf9cc3626de1f"
STREAM = "crates/horizon-client-render/src/streaming.rs"
STREAM_BEFORE = "d51440267f6969f23bc74050d5c2b0305b932ebebd33d2f8c2ffcca5ddb8a6c2"
STREAM_BLOB = "1d492e5b05fab1ea91ad80cf97bcb1f4a7be395f"
PATCH_SHA = "7dc1cff91941e3b1f2c059f01e8379cd1329ca073c42af8b8c0018cd398ce861"
STREAM_AFTER = "526145b17f019e1e1af47014ad2d51f4dac23b2b7bb171137a27bbcc748b4de4"
MAX_FILE = 32 * 1024 * 1024
MAX_READ = 256 * 1024 * 1024
MAX_ENTRIES = 20_000
MAX_DEPTH = 32
SHA = re.compile(r"[0-9a-f]{64}\Z")
DEVICE = re.compile(r"(?:con|prn|aux|nul|com[1-9¹²³]|lpt[1-9¹²³])(?:\..*)?\Z", re.I)


class PreparationError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise PreparationError(message)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def component(name: str) -> None:
    require(
        bool(name)
        and name not in (".", "..")
        and name == unicodedata.normalize("NFC", name)
        and not name.endswith((".", " "))
        and not name.startswith(" ")
        and not DEVICE.fullmatch(name)
        and not any(ord(c) < 32 or c in '/\\:<>"|?*' for c in name),
        "unsafe or aliased path component",
    )


def blocked(metadata: os.stat_result) -> bool:
    return stat.S_ISLNK(metadata.st_mode) or bool(
        getattr(metadata, "st_file_attributes", 0) & 0x400
    )


def ordinary(metadata: os.stat_result, directory: bool) -> None:
    require(not blocked(metadata), "links and reparse points are not supported")
    if directory:
        require(stat.S_ISDIR(metadata.st_mode), "expected an ordinary directory")
    else:
        require(stat.S_ISREG(metadata.st_mode), "expected an ordinary file")
        require(metadata.st_nlink == 1, "hard-linked inputs are not supported")
        require(metadata.st_size <= MAX_FILE, "input file exceeds the size bound")


def exact_child(parent: Path, name: str) -> bool:
    """Refuse case/Unicode aliases, including existing output aliases."""
    component(name)
    found = False
    with os.scandir(parent) as entries:
        for count, entry in enumerate(entries, 1):
            require(count <= MAX_ENTRIES, "directory entry bound exceeded")
            if unicodedata.normalize("NFC", entry.name).casefold() == name.casefold():
                require(entry.name == name, "case or Unicode path alias")
                found = True
    return found


def absolute(path: Path) -> Path:
    require(".." not in path.parts, "parent traversal is not supported")
    return Path(os.path.abspath(path))


def checked_existing(path: Path, *, directory: bool) -> Path:
    path = absolute(path)
    current = Path(path.anchor)
    ordinary(current.lstat(), True)
    for number, name in enumerate(path.parts[1:], 1):
        require(exact_child(current, name), "required input is missing")
        current /= name
        ordinary(current.lstat(), directory or number < len(path.parts) - 1)
    return path


def new_output(path: Path) -> Path:
    path = absolute(path)
    require(path != Path(path.anchor), "output must be a new directory")
    checked_existing(path.parent, directory=True)
    require(not exact_child(path.parent, path.name), "output already exists")
    return path


def walk(root: Path) -> dict[str, bool]:
    """Bound metadata traversal; values distinguish directories from files."""
    checked_existing(root, directory=True)
    result: dict[str, bool] = {}
    aliases: set[str] = set()
    pending = [(root, "", 0)]
    while pending:
        folder, prefix, depth = pending.pop()
        ordinary(folder.lstat(), True)
        with os.scandir(folder) as entries:
            for entry in entries:
                require(len(result) < MAX_ENTRIES, "input tree entry bound exceeded")
                component(entry.name)
                relative = prefix + entry.name
                require(len(relative) <= 512 and depth < MAX_DEPTH, "input path bound exceeded")
                key = relative.casefold()
                require(key not in aliases, "input tree contains aliases")
                aliases.add(key)
                # Windows DirEntry.stat may report zero for inode/link fields.
                metadata = Path(entry.path).lstat()
                directory = stat.S_ISDIR(metadata.st_mode)
                ordinary(metadata, directory)
                result[relative] = directory
                if directory:
                    pending.append((Path(entry.path), relative + "/", depth + 1))
    return result


class Reader:
    def __init__(self) -> None:
        self.total = 0

    def read(self, path: Path, limit: int = MAX_FILE) -> bytes:
        checked_existing(path, directory=False)
        before = path.lstat()
        require(before.st_size <= limit, "input exceeds its size bound")
        with path.open("rb") as handle:
            opened = os.fstat(handle.fileno())
            ordinary(opened, False)
            require(
                (before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino),
                "input changed while opening",
            )
            data = handle.read(limit + 1)
        checked_existing(path, directory=False)
        after = path.lstat()
        require(
            len(data) == before.st_size
            and (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
            == (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns),
            "input changed while reading",
        )
        self.total += len(data)
        require(self.total <= MAX_READ, "total input byte bound exceeded")
        return data


def relative(value: object) -> str:
    require(isinstance(value, str) and len(value) <= 512, "invalid target path")
    parts = value.split("/")
    require(len(parts) <= MAX_DEPTH and parts[0] == "crates", "invalid target fence")
    for part in parts:
        component(part)
        require(re.fullmatch(r"[A-Za-z0-9_-][A-Za-z0-9_.-]*", part) is not None, "invalid target name")
    require(Path(value).suffix in (".rs", ".json", ".hex"), "unsupported target type")
    return value


def unique_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON key")
        result[key] = value
    return result


def blob(data: bytes) -> str:
    return hashlib.sha1(
        b"blob " + str(len(data)).encode("ascii") + b"\0" + data,
        usedforsecurity=False,
    ).hexdigest()


def apply_closed_patch(canonical: bytes, patch: bytes) -> bytes:
    """Apply this one addition-only full-index unified diff without fuzz."""
    require(digest(canonical) == STREAM_BEFORE and blob(canonical) == STREAM_BLOB, "streaming preimage mismatch")
    require(b"\r" not in canonical and b"\r" not in patch, "patch requires canonical LF text")
    source = canonical.splitlines(keepends=True)
    lines = patch.splitlines(keepends=True)
    require(all(line.endswith(b"\n") for line in source + lines), "unterminated patch or source line")
    require(len(lines) >= 5, "incomplete patch")
    target = STREAM.encode("ascii")
    require(lines[0] == b"diff --git a/" + target + b" b/" + target + b"\n", "unexpected patch target")
    index = re.fullmatch(rb"index ([0-9a-f]{40})\.\.([0-9a-f]{40}) 100644\n", lines[1])
    require(index is not None and index[1].decode() == STREAM_BLOB, "invalid patch blob header")
    require(lines[2] == b"--- a/" + target + b"\n" and lines[3] == b"+++ b/" + target + b"\n", "invalid patch file headers")
    output: list[bytes] = []
    cursor = 0
    position = 4
    hunks = 0
    while position < len(lines):
        header = re.fullmatch(rb"@@ -(\d+),(\d+) \+(\d+),(\d+) @@[^\n]*\n", lines[position])
        require(header is not None, "invalid patch hunk header")
        old_start, old_count, new_start, new_count = map(int, header.groups())
        require(old_count > 0 and new_count > old_count, "patch must add lines with context")
        start = old_start - 1
        require(cursor <= start <= len(source), "overlapping or misplaced patch hunk")
        output.extend(source[cursor:start])
        require(len(output) == new_start - 1, "incorrect new hunk position")
        cursor = start
        consumed = emitted = 0
        position += 1
        while position < len(lines) and not lines[position].startswith(b"@@ "):
            line = lines[position]
            if line.startswith(b" "):
                require(cursor < len(source) and source[cursor] == line[1:], "patch context mismatch")
                output.append(source[cursor])
                cursor += 1
                consumed += 1
            else:
                require(line.startswith(b"+"), "patch contains a deletion or unsupported record")
                output.append(line[1:])
            emitted += 1
            position += 1
        require(consumed == old_count and emitted == new_count, "patch hunk count mismatch")
        hunks += 1
    require(hunks > 0, "empty patch")
    output.extend(source[cursor:])
    result = b"".join(output)
    require(digest(result) == STREAM_AFTER and blob(result) == index[2].decode(), "patched streaming source mismatch")
    return result


def reserve(path: str, tree: dict[str, bool], targets: set[str]) -> None:
    key = path.casefold()
    require(key not in targets, "duplicate or aliased target")
    require(not any(key.startswith(other + "/") or other.startswith(key + "/") for other in targets), "target file/directory collision")
    folded = {name.casefold(): name for name in tree}
    parts = path.split("/")
    for end in range(1, len(parts) + 1):
        prefix = "/".join(parts[:end])
        if prefix.casefold() in folded:
            require(folded[prefix.casefold()] == prefix, "target aliases a pristine path")
            if end < len(parts):
                require(tree[prefix], "target parent is a file")
    targets.add(key)


def prepare(pristine: Path, out: Path) -> tuple[dict[str, bytes], bytes, Path]:
    script = checked_existing(Path(__file__), directory=False)
    require(script.parts[-3:] == ("tools", "bedrock", "prepare_source_overlay.py"), "tool must retain its repository location")
    root = script.parents[2]
    pristine = checked_existing(pristine, directory=True)
    out = new_output(out)
    legacy = root / "mixins/horizon_via/source_patch/data"
    host = root / "bedrock/host"
    for protected in (pristine, legacy, host, script.parent):
        require(not out.is_relative_to(protected), "output overlaps authoring inputs")
    baseline = walk(pristine)
    inputs = walk(legacy)
    reader = Reader()
    marker = reader.read(pristine / ".horizon-revision", 128)
    require(marker in (BASE.encode(), (BASE + "\n").encode(), (BASE + "\r\n").encode()), "wrong pristine revision marker")
    document = json.loads(reader.read(legacy / "overlay.json", 1024 * 1024).decode("utf-8"), object_pairs_hook=unique_object)
    require(isinstance(document, dict) and set(document) == {"schema", "horizon_revision", "files"}, "invalid legacy index shape")
    require(type(document["schema"]) is int and document["schema"] == 1 and document["horizon_revision"] == BASE, "wrong legacy index revision or schema")
    rows = document["files"]
    require(isinstance(rows, list) and len(rows) == 125, "expected 125 legacy rows")
    prepared = []
    payloads: dict[str, bytes] = {}
    targets: set[str] = set()
    expected_inputs = {"overlay.json"}
    replacements = additions = 0
    for row in rows:
        require(isinstance(row, dict) and set(row) == {"path", "before_sha256", "after_sha256"}, "invalid legacy row")
        path = relative(row["path"])
        reserve(path, baseline, targets)
        before, after = row["before_sha256"], row["after_sha256"]
        require(isinstance(after, str) and SHA.fullmatch(after) is not None, "invalid payload digest")
        require(before is None or isinstance(before, str) and SHA.fullmatch(before) is not None, "invalid preimage digest")
        payload_name = "payload/" + path
        expected_inputs.add(payload_name)
        payload = reader.read(legacy / payload_name)
        require(digest(payload) == after, "legacy payload digest mismatch")
        raw_before = None
        if before is None:
            require(path not in baseline, "new target already exists")
            additions += 1
        else:
            require(path in baseline and not baseline[path], "replacement target is missing")
            original = reader.read(pristine / path)
            require(digest(original.replace(b"\r\n", b"\n")) == before, "legacy normalized preimage mismatch")
            raw_before = digest(original)
            replacements += 1
        destination = "data/source-overlay/payload/" + path
        payloads[destination] = payload
        prepared.append({"path": path, "payload": destination, "before_sha256": raw_before, "after_sha256": after})
    require({name for name, directory in inputs.items() if not directory} == expected_inputs, "unindexed legacy input")
    require((replacements, additions) == (30, 95), "unexpected legacy replacement/new counts")
    reserve(STREAM, baseline, targets)
    require(STREAM in baseline and not baseline[STREAM], "streaming preimage is missing")
    original = reader.read(pristine / STREAM)
    patch = reader.read(host / "streaming-unlit.patch", 1024 * 1024)
    require(digest(patch) == PATCH_SHA, "reviewed streaming patch digest mismatch")
    streaming = apply_closed_patch(original.replace(b"\r\n", b"\n"), patch)
    destination = "data/source-overlay/payload/" + STREAM
    payloads[destination] = streaming
    prepared.append({"path": STREAM, "payload": destination, "before_sha256": digest(original), "after_sha256": digest(streaming)})
    index = json.dumps({"schema": 1, "base_revision": BASE, "files": prepared}, sort_keys=True, separators=(",", ":")).encode("utf-8")
    payloads["data/source-overlay/index.json"] = index
    snippet = ('[source_overlay]\nindex = "data/source-overlay/index.json"\nsha256 = "' + digest(index) + '"\n').encode("ascii")
    return payloads, snippet, out


def write_new(out: Path, files: dict[str, bytes], snippet: bytes) -> None:
    new_output(out)
    out.mkdir(mode=0o700)
    for name, data in {**files, "source-overlay.toml": snippet}.items():
        destination = out / name
        current = out
        for part in destination.relative_to(out).parts[:-1]:
            ordinary(current.lstat(), True)
            child = current / part
            if not exact_child(current, part):
                child.mkdir(mode=0o700)
            ordinary(child.lstat(), True)
            current = child
        checked_existing(destination.parent, directory=True)
        require(not exact_child(destination.parent, destination.name), "output file already exists")
        with destination.open("xb") as handle:
            handle.write(data)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pristine", required=True, type=Path, help="isolated exact-d8 export")
    parser.add_argument("--out", required=True, type=Path, help="new output directory; parent must exist")
    args = parser.parse_args()
    try:
        files, snippet, out = prepare(args.pristine, args.out)
        write_new(out, files, snippet)
    except (ValueError, OSError, RecursionError, OverflowError):
        print("error: source-overlay preparation refused; inputs, paths or output did not validate", file=sys.stderr)
        return 1
    print("Prepared 126 source payloads, a schema-1 index and source-overlay.toml.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
