"""Exercise root gating and failure ordering at the pacman-key boundary."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ENTRY = Path(__file__).resolve().parents[2] / 'distro/wsl-operator/rootfs/usr/lib/archie/init-pacman-keyring'


class KeyringInit(unittest.TestCase):
    def invoke(self, uid='0', fail_init=False):
        self.assertTrue(ENTRY.is_file(), 'missing per-install keyring initializer')
        bash = shutil.which('bash')
        self.assertIsNotNone(bash)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'id').write_text('#!/bin/sh\necho '+uid+'\n', newline='\n')
            (root / 'pacman-key').write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$KEYRING_TEST_TRACE"\n'
                + ('[ "$1" != --init ] || exit 7\n' if fail_init else ''), newline='\n'
            )
            for path in root.iterdir(): path.chmod(0o755)
            trace = root / 'trace'
            # bash receives a POSIX PATH; passing a Windows semicolon PATH to it
            # would prevent the boundary commands from being resolved.
            def posix(path):
                value = str(path).replace('\\', '/')
                return '/'+value[0].lower()+value[2:] if len(value)>1 and value[1]==':' else value
            env = dict(os.environ, KEYRING_TEST_TRACE=posix(trace))
            result = subprocess.run([bash, '-c', 'export PATH="$1:/usr/bin:/bin"; exec /usr/bin/bash "$2"',
                                     'keyring-test', posix(root), posix(ENTRY)], env=env,
                                    capture_output=True, text=True, timeout=20)
            calls = trace.read_text().splitlines() if trace.exists() else []
            return result, calls

    def test_non_root_never_initializes(self):
        result, calls = self.invoke(uid='1000')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])

    def test_init_failure_stops_before_population(self):
        result, calls = self.invoke(fail_init=True)
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertEqual(calls, ['--init'])

    def test_initializer_populates_installed_public_arch_keyring(self):
        result, calls = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ['--init', '--populate archlinux'])


if __name__ == '__main__':
    unittest.main()
