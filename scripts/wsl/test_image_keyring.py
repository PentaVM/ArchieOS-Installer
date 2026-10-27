"""The staging guard must reject keys and cannot clean outside its rootfs."""
import importlib.util
import pathlib
import os
import subprocess
import tempfile
import unittest

ENTRY = pathlib.Path(__file__).with_name('image-keyring.py')


class ImageKeyring(unittest.TestCase):
    def setUp(self):
        self.assertTrue(ENTRY.is_file(), 'missing staged keyring guard')
        spec = importlib.util.spec_from_file_location('image_keyring', ENTRY)
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name) / 'rootfs'
        (self.root / 'etc').mkdir(parents=True)
        (self.root / 'etc/os-release').write_text('ID=arch\n')
        self.ring = self.root / 'etc/pacman.d/gnupg'
        self.ring.mkdir(parents=True)

    def test_builder_identity_and_trust_cannot_enter_image(self):
        private = self.ring / 'private-keys-v1.d'
        private.mkdir()
        (private / 'builder.key').write_text('private fixture')
        (self.ring / 'trustdb.gpg').write_bytes(b'old trust')
        with self.assertRaisesRegex(ValueError, 'keyring'):
            self.module.check_image(self.root)
        self.module.prepare_image(self.root)
        self.module.check_image(self.root)
        self.assertEqual(list(self.ring.iterdir()), [])

    def test_public_ring_without_private_key_is_also_rejected(self):
        (self.ring / 'pubring.kbx').write_bytes(b'inherited identity')
        with self.assertRaisesRegex(ValueError, 'keyring'):
            self.module.check_image(self.root)

    def test_live_filesystem_root_is_refused(self):
        with self.assertRaisesRegex(ValueError, 'staging'):
            self.module.prepare_image(pathlib.Path(self.root.anchor))

    def test_symlinked_keyring_cannot_delete_outside_files(self):
        outside = self.root.parent / 'outside'
        outside.mkdir()
        sentinel = outside / 'sentinel'
        sentinel.write_text('keep')
        self.ring.rmdir()
        try:
            self.ring.symlink_to(outside, target_is_directory=True)
        except OSError:
            if os.name != 'nt':
                self.skipTest('directory symlinks unavailable')
            result = subprocess.run(['cmd', '/c', 'mklink', '/J', str(self.ring), str(outside)],
                                    capture_output=True, text=True)
            if result.returncode:
                self.skipTest('directory links unavailable')
        with self.assertRaisesRegex(ValueError, 'symlink|junction|escape'):
            self.module.prepare_image(self.root)
        self.assertEqual(sentinel.read_text(), 'keep')

    def test_absent_keyring_is_safe(self):
        self.ring.rmdir()
        self.module.check_image(self.root)


if __name__ == '__main__':
    unittest.main()
