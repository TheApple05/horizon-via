"""Build deterministic, source-only Horizon native mod and source archives."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def canonical(data: bytes) -> bytes:
    return data.replace(b'\r\n', b'\n')


def sha256(data: bytes) -> str:
    return hashlib.sha256(canonical(data)).hexdigest()


def validate() -> tuple[dict, list[Path]]:
    metadata = json.loads((ROOT / 'horizon-native.json').read_text(encoding='utf-8'))
    assert metadata['schema'] == 1 and metadata['id'] == 'horizon_via'
    for key in ('horizon', 'mixins'):
        assert re.fullmatch(r'[0-9a-f]{40}', metadata[key]), key
    bundle = ROOT / 'mixins' / metadata['id']
    toml = tomllib.loads((bundle / 'horizon-mixin.toml').read_text(encoding='utf-8'))
    assert toml['mod']['id'] == metadata['id']
    assert toml['mod']['version'] == metadata['version']
    assert toml['compatibility']['horizon_revision'] == metadata['horizon']
    crate = bundle / 'source_patch'
    index = json.loads((crate / 'data/overlay.json').read_text(encoding='utf-8'))
    assert index['schema'] == 1 and index['horizon_revision'] == metadata['horizon']
    seen = set()
    for entry in index['files']:
        path = entry['path']
        assert path not in seen and '\\' not in path and ':' not in path
        assert not Path(path).is_absolute() and all(p not in ('', '.', '..') for p in path.split('/'))
        assert path.startswith('crates/')
        seen.add(path)
        data = (crate / 'data/payload' / path).read_bytes()
        assert sha256(data) == entry['after_sha256'], path
        assert entry['before_sha256'] is None or re.fullmatch(r'[0-9a-f]{64}', entry['before_sha256'])
        # The source overlay must never depend on personal launcher integration.
        assert b'local_profile' not in data and b'prism_accounts_file' not in data, path
        assert not re.search(rb'(?:[A-Z]:[\\/]+Users[\\/]|/home/[^/]+/)', data), path
    files = [ROOT / 'horizon-native.json'] + sorted(p for p in bundle.rglob('*') if p.is_file())
    assert len(files) <= 512 and sum(p.stat().st_size for p in files) <= 64 * 1024 * 1024
    for path in files:
        assert not path.is_symlink()
        rest = path.relative_to(bundle).as_posix() if path != files[0] else None
        assert rest is None or path.suffix in ('.rs', '.toml') or rest.startswith('source_patch/data/') or path.name.startswith(('LICENSE', 'README', 'NOTICE'))
    return metadata, files


def archive(path: Path, files: list[tuple[str, Path]]) -> None:
    with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
        for name, source in sorted(files):
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            output.writestr(info, canonical(source.read_bytes()))


def main() -> None:
    # Native archives allow README/NOTICE names beside their source bundle.
    bundle = ROOT / 'mixins/horizon_via'
    readme = (ROOT / 'README.md').read_text(encoding='utf-8')
    readme = readme.replace('](PROVENANCE.md)', '](NOTICE-PROVENANCE.md)')
    readme = readme.replace('](CHANGELOG.md)', '](README-VALIDATION.md)')
    (bundle / 'README.md').write_text(readme, encoding='utf-8', newline='\n')
    (bundle / 'NOTICE-PROVENANCE.md').write_bytes((ROOT / 'PROVENANCE.md').read_bytes())
    (bundle / 'README-VALIDATION.md').write_bytes((ROOT / 'CHANGELOG.md').read_bytes())
    metadata, files = validate()
    destination = ROOT / 'dist'
    destination.mkdir(exist_ok=True)
    stem = f"horizon-via-{metadata['version']}"
    mod = destination / f'{stem}.hnmod'
    archive(mod, [(p.relative_to(ROOT).as_posix(), p) for p in files])
    root_files = ['README.md', 'LICENSE', 'PROVENANCE.md', 'CHANGELOG.md', '.gitignore', 'horizon-native.json']
    source_files = [(p.relative_to(ROOT).as_posix(), p) for p in files if p.name != 'horizon-native.json']
    source_files += [(name, ROOT / name) for name in root_files]
    source_files += [('tools/package.py', Path(__file__).resolve())]
    source = destination / f'{stem}-source.zip'
    archive(source, source_files)
    checksums = ''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in (mod, source))
    (destination / 'SHA256SUMS.txt').write_text(checksums, encoding='ascii')
    print(json.dumps(dict(artifact=mod.name, entries=len(files), bytes=mod.stat().st_size,
                          sha256=hashlib.sha256(mod.read_bytes()).hexdigest(), source=source.name)))


if __name__ == '__main__':
    main()
