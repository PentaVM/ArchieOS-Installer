"""Exercise the operator's real packaging tail on tiny temporary rootfs fixtures."""
import os
import shutil
from pathlib import Path
import subprocess
import tempfile
import tarfile
import sys
import unittest

REPO = Path(__file__).resolve().parents[2]


class OperatorPackaging(unittest.TestCase):
    def package(self, split, inherited_key=False):
        script = (REPO / 'scripts/wsl/build-operator.sh').read_text()
        tail = script[script.index('# Final image packaging.'):]
        if split:
            tail = tail.replace('1900 * 1024 * 1024', '1024').replace('1900M', '1K')
        with tempfile.TemporaryDirectory(prefix='archie-operator-pack-test-') as temp:
            root = Path(temp)
            rootfs = root / 'rootfs'
            rootfs.mkdir()
            (rootfs / 'etc').mkdir()
            (rootfs / 'etc/os-release').write_text('ID=arch\n')
            (rootfs / 'fixture').write_bytes(os.urandom(8192))
            if inherited_key:
                ring = rootfs / 'etc/pacman.d/gnupg'
                (ring / 'private-keys-v1.d').mkdir(parents=True)
                (ring / 'private-keys-v1.d/builder.key').write_text('private fixture')
                (ring / 'trustdb.gpg').write_text('builder trust')
            out = root / 'out'
            env = dict(os.environ, rootfs=str(rootfs), out=str(out), repo_root=str(REPO),
                       ARCHIE_TEST_PYTHON=sys.executable.replace('\\', '/'))
            command = 'python3() { "$ARCHIE_TEST_PYTHON" "$@"; }\n' + tail
            result = subprocess.run([shutil.which('bash'), '-euo', 'pipefail', '-c', command], env=env,
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((out / 'SHA256SUMS').is_file())
            self.assertTrue((out / 'install-archie-operator.ps1').is_file())
            if split:
                self.assertFalse((out / 'ArchieOS-Operator.wsl').exists())
                self.assertGreater(len(list(out.glob('ArchieOS-Operator.wsl.[0-9][0-9][0-9]'))), 1)
            else:
                self.assertTrue((out / 'ArchieOS-Operator.wsl').is_file())
                with tarfile.open(out / 'ArchieOS-Operator.wsl') as archive:
                    self.assertFalse(any(name.endswith(('builder.key', 'trustdb.gpg'))
                                         for name in archive.getnames()))

    def test_whole_image(self):
        self.package(False)

    def test_split_image(self):
        self.package(True)

    def test_builder_key_and_trust_do_not_enter_archive(self):
        self.package(False, inherited_key=True)


if __name__ == '__main__':
    unittest.main()
