"""Exercise the operator's real packaging tail on tiny temporary rootfs fixtures."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class OperatorPackaging(unittest.TestCase):
    def package(self, split):
        script = (REPO / 'scripts/wsl/build-operator.sh').read_text()
        tail = script[script.index('out_dir=$(realpath -m "$out")'):]
        if split:
            tail = tail.replace('1900 * 1024 * 1024', '1024').replace('1900M', '1K')
        with tempfile.TemporaryDirectory(prefix='archie-operator-pack-test-') as temp:
            root = Path(temp)
            rootfs = root / 'rootfs'
            rootfs.mkdir()
            (rootfs / 'fixture').write_bytes(os.urandom(8192))
            out = root / 'out'
            env = dict(os.environ, rootfs=str(rootfs), out=str(out), repo_root=str(REPO))
            result = subprocess.run(['bash', '-euo', 'pipefail', '-c', tail], env=env,
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((out / 'SHA256SUMS').is_file())
            self.assertTrue((out / 'install-archie-operator.ps1').is_file())
            if split:
                self.assertFalse((out / 'ArchieOS-Operator.wsl').exists())
                self.assertGreater(len(list(out.glob('ArchieOS-Operator.wsl.[0-9][0-9][0-9]'))), 1)
            else:
                self.assertTrue((out / 'ArchieOS-Operator.wsl').is_file())

    def test_whole_image(self):
        self.package(False)

    def test_split_image(self):
        self.package(True)


if __name__ == '__main__':
    unittest.main()
