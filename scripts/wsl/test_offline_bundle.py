"""Packaging proofs; tiny files exercise integrity, never model quality or hardware."""
import hashlib, importlib.util, json, os, pathlib, subprocess, tempfile, unittest

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

    def run_oobe(self, runuser_script):
        if not pathlib.Path('/bin/bash').exists(): self.skipTest('Linux shell integration')
        bindir = self.root / 'bin'; bindir.mkdir()
        for name, script in [('install', 'exit 0'), ('pacman-key', 'exit 0'), ('runuser', runuser_script)]:
            path = bindir / name; path.write_text('#!/bin/sh\n' + script + '\n'); path.chmod(0o755)
        oobe = ENTRY.parent.parent / 'wsl/archie-offline-oobe'
        return subprocess.run(['/bin/bash', str(oobe)], env=dict(os.environ, PATH=str(bindir)+':/usr/bin:/bin'), capture_output=True, text=True)

    def test_oobe_preserves_install_when_desktop_cannot_start(self):
        result = self.run_oobe('case "$*" in *--check*) exit 0;; esac\necho "port occupied" >&2\nexit 7')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_oobe_still_rejects_invalid_payload(self):
        result = self.run_oobe('exit 9')
        self.assertNotEqual(result.returncode, 0)

    def test_stale_launcher_in_image_is_rejected(self):
        spec = importlib.util.spec_from_file_location('overlay', pathlib.Path(__file__).with_name('validate-offline-overlay.py'))
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        overlay = self.root / 'source'; path = overlay / 'usr/bin/archie-design'
        path.parent.mkdir(parents=True); path.write_text('native entry')
        target = self.root / 'usr/bin/archie-design'; target.parent.mkdir(parents=True); target.write_text('old preview')
        with self.assertRaisesRegex(ValueError, 'archie-design'):
            module.verify_overlay(overlay, self.root)

    def test_current_launcher_in_image_is_accepted(self):
        spec = importlib.util.spec_from_file_location('overlay', pathlib.Path(__file__).with_name('validate-offline-overlay.py'))
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        overlay = self.root / 'source'; path = overlay / 'usr/bin/archie-design'
        path.parent.mkdir(parents=True); path.write_text('native entry')
        target = self.root / 'usr/bin/archie-design'; target.parent.mkdir(parents=True); target.write_text('native entry')
        module.verify_overlay(overlay, self.root)

    def profile(self, text):
        path = self.root / 'profile.json'
        path.write_text(text)
        return self.module.load_profile(path)

    def test_profile_defaults_listen_only_when_called(self):
        self.assertEqual(self.module.load_profile(self.root / 'absent.json'), self.module.PROFILE_DEFAULTS)
        self.assertEqual(self.module.PROFILE_DEFAULTS['listen'], 'named')
        self.assertFalse(self.module.PROFILE_DEFAULTS['wanda'])

    def test_profile_keeps_valid_choices_and_drops_the_rest(self):
        chosen = self.profile('{"asr_language":"en-US","listen":"always","voice":false,"wanda":true,"headphones":true}')
        self.assertEqual(chosen, {'asr_language': 'en-US', 'listen': 'always', 'voice': False, 'wanda': True, 'headphones': True})
        bad = self.profile('{"asr_language":"english; rm -rf /","listen":"yes","voice":"no","wanda":1}')
        self.assertEqual(bad, self.module.PROFILE_DEFAULTS)
        self.assertEqual(self.profile('not json'), self.module.PROFILE_DEFAULTS)
        self.assertEqual(self.profile('[1, 2]'), self.module.PROFILE_DEFAULTS)
        self.assertEqual(self.profile('{"asr_language":"auto"}')['asr_language'], 'auto')

    def test_voice_command_changes_only_what_the_profile_asks(self):
        models = {k: {'path': '/m/' + k} for k in ['asr', 'piper_es', 'piper_en']}
        base = self.module.voice_arguments(self.module.PROFILE_DEFAULTS, models)
        self.assertNotIn('--always', base)
        self.assertNotIn('--listen-language', base)
        self.assertNotIn('--headphones', base)
        custom = dict(self.module.PROFILE_DEFAULTS, asr_language='en-US', listen='always', headphones=True)
        command = self.module.voice_arguments(custom, models)
        self.assertEqual(command[command.index('--listen-language') + 1], 'en-US')
        self.assertIn('--always', command)
        self.assertIn('--headphones', command)
        self.assertEqual(command[:len(base)], base)

if __name__ == '__main__': unittest.main()
