"""The distributable payload must match its reviewed source index exactly."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('mod_package', Path(__file__).with_name('package.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class SourceIndexTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.original = package.ROOT
        package.ROOT = self.root
        self.addCleanup(setattr, package, 'ROOT', self.original)
        self.bundle = self.root / 'mixins/horizon_via'
        self.data = self.bundle / 'source_patch/data'
        self.relative = 'crates/horizon-protocol/src/adapter.rs'
        self.payload = self.data / 'payload' / self.relative
        self.payload.parent.mkdir(parents=True)
        self.payload.write_bytes(b'// Source fixture\n')
        metadata = dict(schema=1, id='horizon_via', version='0.0.0', horizon='a' * 40, mixins='b' * 40)
        (self.root / 'horizon-native.json').write_text(json.dumps(metadata), encoding='utf-8')
        (self.bundle / 'horizon-mixin.toml').write_text(
            '[mod]\nid="horizon_via"\nversion="0.0.0"\n[compatibility]\nhorizon_revision="' + 'a' * 40 + '"\n',
            encoding='utf-8')
        self.index = dict(schema=1, horizon_revision='a' * 40, files=[dict(
            path=self.relative, before_sha256=None, after_sha256=package.sha256(self.payload.read_bytes()))])
        self.save_index()

    def save_index(self):
        (self.data / 'overlay.json').write_text(json.dumps(self.index), encoding='utf-8')

    def test_reviewed_source_is_accepted_and_unindexed_source_is_rejected(self):
        package.validate()
        self.payload.with_name('unreviewed.rs').write_bytes(b'// Outside the reviewed index\n')
        with self.assertRaisesRegex(AssertionError, 'exact hash index'):
            package.validate()

    def test_windows_case_collision_is_rejected_before_reading_an_alias(self):
        alias = dict(self.index['files'][0], path=self.relative.upper())
        self.index['files'].append(alias)
        self.save_index()
        with self.assertRaises(AssertionError):
            package.validate()

    def test_a_missing_or_modified_reviewed_source_cannot_be_packaged(self):
        self.payload.write_bytes(b'// Different source\n')
        with self.assertRaises(AssertionError):
            package.validate()
        self.payload.unlink()
        with self.assertRaises(FileNotFoundError):
            package.validate()


if __name__ == '__main__':
    unittest.main()
