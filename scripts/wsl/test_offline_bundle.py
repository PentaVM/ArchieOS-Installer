"""Packaging proofs; tiny files exercise integrity, never model quality or hardware."""
import hashlib, importlib.util, json, pathlib, tempfile, unittest

ENTRY = pathlib.Path(__file__).resolve().parents[2] / 'distro/wsl-operator/rootfs/usr/lib/archie/offline-session.py'

class OfflineBundle(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('offline_session', ENTRY)
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.files = {}
        for role in ['bonsai', 'asr', 'piper_es', 'piper_en', 'iris']:
            rel = '/var/lib/archie/models/' + role
            path = self.root / rel.lstrip('/')
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(role.encode())
            self.files[role] = {'path': rel, 'bytes': path.stat().st_size, 'sha256': hashlib.sha256(role.encode()).hexdigest()}
        self.manifest = self.root / 'usr/share/archie/offline/payload.json'
        self.manifest.parent.mkdir(parents=True)
        self.write_manifest()

    def write_manifest(self):
        self.manifest.write_text(json.dumps({'format': 1, 'source_sha': 'a'*40, 'files': self.files}))

    def test_complete_bundle_checks_actual_bytes(self):
        self.assertEqual(self.module.verify_bundle(self.root)['source_sha'], 'a'*40)

    def test_missing_required_model_is_rejected(self):
        del self.files['iris']; self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'iris'):
            self.module.verify_bundle(self.root)

    def test_corrupted_model_is_rejected(self):
        (self.root / self.files['bonsai']['path'].lstrip('/')).write_bytes(b'broken')
        with self.assertRaisesRegex(ValueError, 'bonsai'):
            self.module.verify_bundle(self.root)

    def test_path_escape_is_rejected(self):
        self.files['bonsai']['path'] = '/../../outside'; self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'path'):
            self.module.verify_bundle(self.root)

    def test_symlink_escape_is_rejected(self):
        if not hasattr(pathlib.Path, 'symlink_to'): self.skipTest('no symlinks')
        path = self.root / self.files['bonsai']['path'].lstrip('/')
        path.unlink()
        try: path.symlink_to(self.root.parent / 'outside')
        except OSError: self.skipTest('symlinks unavailable')
        with self.assertRaisesRegex(ValueError, 'path'):
            self.module.verify_bundle(self.root)

if __name__ == '__main__': unittest.main()
