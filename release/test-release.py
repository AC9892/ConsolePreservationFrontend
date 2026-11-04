import importlib.util, tempfile, unittest, zipfile
from pathlib import Path
spec = importlib.util.spec_from_file_location('prepare_public', Path(__file__).with_name('prepare-public.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
TEST_ROOT = release.PROJECT/'release-staging'/'audit-tests'
TEST_ROOT.mkdir(parents=True, exist_ok=True)

class UploadAuditTests(unittest.TestCase):
    def test_rejects_private_paths_in_text_and_executable_encodings(self):
        with tempfile.TemporaryDirectory(dir=TEST_ROOT) as tmp:
            root = Path(tmp)
            path = root/'probe.exe'
            for encoding in ['utf-8','utf-16le']:
                path.write_bytes(str(release.PROJECT).encode(encoding))
                with self.assertRaises(RuntimeError): release.audit(root)
    def test_rejects_console_payloads_and_private_directories(self):
        with tempfile.TemporaryDirectory(dir=TEST_ROOT) as tmp:
            root = Path(tmp)
            for name in ['game.cia','nand.bin','key.pem','private/profile.toml']:
                path = root/name
                path.parent.mkdir(exist_ok=True)
                path.write_bytes(b'payload')
                with self.assertRaises(RuntimeError): release.audit(root)
                path.unlink()
    def test_archive_supports_old_dependency_timestamps_and_manifest(self):
        import os
        with tempfile.TemporaryDirectory(dir=TEST_ROOT) as tmp:
            root = Path(tmp)/'source'; root.mkdir()
            path=root/'hello.rs'; path.write_text('fn main() {}')
            os.utime(path, (0,0))
            release.manifest(root)
            self.assertEqual(release.audit(root),2)
            output=Path(tmp)/'source.zip'
            release.archive(root,output)
            with zipfile.ZipFile(output) as z:
                self.assertEqual(set(z.namelist()),{'hello.rs','MANIFEST.sha256'})
                self.assertIsNone(z.testzip())

if __name__=='__main__': unittest.main()
