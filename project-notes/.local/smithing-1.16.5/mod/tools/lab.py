"""Minimal report-generator input helpers; players do not need these tools."""
import json
from pathlib import Path
import zipfile


def version_info(jar: Path | None = None) -> dict:
    if jar is None:
        raise ValueError('Pass an explicit owned vanilla reference jar')
    with zipfile.ZipFile(jar) as archive:
        return json.loads(archive.read('version.json'))
