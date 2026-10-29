#!/usr/bin/env python3
"""Explicit local desktop launch. No downloads, cloud fallback, or Stage authority."""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path, PurePosixPath
import secrets
import socket
import subprocess
import time
import urllib.request

# What the image carries. The reasoning model is not among them: the person chooses it when
# installing (the launcher's AI model, archie-local setup) and archie-local serves it.
REQUIRED = {'asr', 'piper_es', 'piper_en', 'iris'}

def local_model():
    """The model archie-local serves: its name, endpoint and alias, from `archie-local current`,
    once it is here. None of them is in the image; without one there is nothing to answer."""
    try:
        models = json.loads(subprocess.run(['archie-local', 'list', '--json'], capture_output=True,
                                           text=True, check=True, timeout=60).stdout)
        name, endpoint, alias = subprocess.run(['archie-local', 'current'], capture_output=True,
                                               text=True, check=True, timeout=30).stdout.split()
    except (OSError, ValueError, subprocess.SubprocessError):
        raise ValueError('archie-local cannot list the local models')
    if not any(m.get('name') == name and m.get('here') for m in models):
        raise ValueError('no AI model is installed yet: choose one in the ArchieOS launcher '
                         '(Install → AI model), or run archie-local setup <name> and archie-local use <name>')
    return name, endpoint, alias

def verify_bundle(prefix=Path('/')):
    prefix = prefix.resolve()
    manifest = json.loads((prefix / 'usr/share/archie/offline/payload.json').read_text())
    if manifest.get('format') != 1:
        raise ValueError('unsupported offline manifest')
    missing = REQUIRED - manifest.get('files', {}).keys()
    if missing:
        raise ValueError('missing required models: ' + ', '.join(sorted(missing)))
    for role, record in manifest['files'].items():
        rel = PurePosixPath(record['path'])
        if not rel.is_absolute() or '..' in rel.parts:
            raise ValueError('invalid payload path: ' + role)
        path = prefix.joinpath(*rel.parts[1:])
        if not path.resolve().is_relative_to(prefix):
            raise ValueError('escaping payload path: ' + role)
        if not path.is_file() or path.stat().st_size != record['bytes']:
            raise ValueError('missing/truncated payload: ' + role)
        with path.open('rb') as source:
            digest = hashlib.file_digest(source, 'sha256').hexdigest()
        if digest != record['sha256']:
            raise ValueError('payload checksum mismatch: ' + role)
    return manifest

PROFILE_DEFAULTS = {'asr_language': 'es-US', 'listen': 'named', 'voice': True, 'wanda': False, 'headphones': False}

def load_profile(path=None):
    """What the person chose in the Windows installer (~/.config/archie/profile.json): the language
    the ASR listens for, whether Archie answers everything it hears or only when called, the voice,
    headphones and Wanda. A missing, unreadable or out-of-range value is the default, so a bad file
    never keeps the desktop from starting."""
    path = path or Path.home() / '.config/archie/profile.json'
    profile = dict(PROFILE_DEFAULTS)
    try:
        raw = json.loads(path.read_text())
    except (OSError, ValueError):
        return profile
    if not isinstance(raw, dict):
        return profile
    language = raw.get('asr_language')
    if isinstance(language, str) and (language == 'auto' or re.fullmatch(r'[a-z]{2}-[A-Z]{2}', language)):
        profile['asr_language'] = language
    if raw.get('listen') in ('named', 'always'):
        profile['listen'] = raw['listen']
    for key in ('voice', 'wanda', 'headphones'):
        if isinstance(raw.get(key), bool):
            profile[key] = raw[key]
    return profile

def voice_arguments(profile, models, endpoint, alias):
    """The archie-voice converse command line for a profile, answered by the chosen local model at
    endpoint (its alias names it there). Always listening is opt-in: by default only what is
    addressed to Archie reaches the floor."""
    command = ['archie-voice', 'converse', '--grade', 'internal', '--endpoint',
               endpoint, '--model', alias, '--timeout-seconds', '600',
               '--budget', 'sentence', '--language', 'es-CL', '--asr-model', models['asr']['path'],
               '--voice-engine', 'piper', '--piper', '/usr/lib/archie/piper-json-bridge.py',
               '--piper-model', models['piper_es']['path'], '--piper-model-en', models['piper_en']['path'],
               '--threads', '2']
    if profile['asr_language'] != PROFILE_DEFAULTS['asr_language']:
        command += ['--listen-language', profile['asr_language']]
    if profile['listen'] == 'always':
        command.append('--always')
    if profile['headphones']:
        command.append('--headphones')
    return command

def identity(pid):
    try:
        # comm may contain spaces or parentheses; fields after its last ')' start at3.
        fields = Path('/proc', str(pid), 'stat').read_text().rsplit(')', 1)[1].split()
        return None if fields[0] == 'Z' else fields[19]
    except (OSError, IndexError):
        return None

def start(manifest, voice=True):
    import fcntl
    profile = load_profile()
    voice = voice and profile['voice']
    if os.geteuid() == 0:
        raise ValueError('run the desktop as archie, never root')
    state = Path.home() / '.local/state/archie-offline'
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(state, 0o700)
    lock = (state / 'launch.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def owned(name):
        try:
            record = json.loads((state / (name + '.json')).read_text())
            return identity(record['pid']) == record['start_ticks']
        except (OSError, ValueError, KeyError):
            return False

    def spawn(name, args, extra=None):
        if owned(name):
            return False
        env = dict(os.environ, ARCHIE_ROOT=str(state / 'node'), ARCHIE_ORB_SOCKET=str(state / 'orb.sock'),
                   LANG='C.UTF-8', LC_ALL='C.UTF-8')
        env.update(extra or {})
        with (state / (name + '.log')).open('ab', buffering=0) as log:
            child = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                                     env=env, start_new_session=True)
        record = {'pid': child.pid, 'start_ticks': identity(child.pid), 'args': args}
        if record['start_ticks'] is None:
            raise ValueError(name + ' exited; inspect ' + str(state / (name + '.log')))
        (state / (name + '.json')).write_text(json.dumps(record))
        return True

    # Check all fixed endpoints before starting anything. Existing unowned services
    # are never replaced, adopted or killed, including another WSL distribution.
    for port, name in [(7666, 'displayd'), (7668, 'display')]:
        with socket.socket() as probe:
            if probe.connect_ex(('127.0.0.1', port)) == 0 and not owned(name):
                raise ValueError(f'loopback port {port} belongs to another service; it was preserved')
    if not Path('/mnt/wslg/runtime-dir/wayland-0').is_socket():
        raise ValueError('WSLg is required for the desktop')
    root = state / 'node'
    (root / 'etc/models').mkdir(parents=True, exist_ok=True)
    data, sock = state / 'postgres', state / 'pgsock'
    sock.mkdir(exist_ok=True, mode=0o700)
    password = state / 'postgres.password'
    if not password.exists():
        fd = os.open(password, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(fd, 'w') as output:
            output.write(secrets.token_urlsafe(32))
    if not (data / 'PG_VERSION').exists():
        subprocess.run(['initdb', '-D', str(data), '--username=archie', '--pwfile=' + str(password),
                        '--auth-local=trust', '--auth-host=scram-sha-256', '--locale=C.UTF-8',
                        '--data-checksums', '--no-instructions'], check=True)
    if subprocess.run(['pg_ctl', '-D', str(data), 'status'], capture_output=True).returncode:
        subprocess.run(['pg_ctl', '-D', str(data), '-l', str(state / 'postgres.log'), '-w', '-t', '30',
                        '-o', "-h '' -k " + str(sock) + ' -c shared_buffers=64MB -c max_connections=30'
                        ' -c shared_preload_libraries=timescaledb -c timescaledb.telemetry_level=off', 'start'], check=True)
    pg = ['-h', str(sock), '-U', 'archie']
    exists = subprocess.check_output(['psql', *pg, '-d', 'postgres', '-Atc',
                                     "SELECT 1 FROM pg_database WHERE datname='archie_offline'"], text=True).strip()
    if exists != '1':
        subprocess.run(['createdb', *pg, 'archie_offline'], check=True)
    subprocess.run(['psql', *pg, '-d', 'archie_offline', '-v', 'ON_ERROR_STOP=1', '-c',
                    "CREATE EXTENSION IF NOT EXISTS timescaledb VERSION '2.29.2';"
                    "CREATE EXTENSION IF NOT EXISTS vector VERSION '0.8.6';"], check=True)
    config = root / 'etc/database.json'
    if not config.exists():
        fd = os.open(config, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(fd, 'w') as output:
            json.dump({'connection': 'host=' + str(sock) + ' user=archie dbname=archie_offline',
                       'schema': 'archie_offline'}, output)
    # The model the person chose, served by archie-local on its own port, and Core's manifest
    # for it, written once from archie-local's catalog.
    name, endpoint, alias = local_model()
    model = root / 'etc/models' / ('local-' + name + '.toml')
    if not model.exists():
        model.write_text(subprocess.run(['archie-local', 'manifest', name], capture_output=True,
                                        text=True, check=True).stdout)
    if subprocess.run(['archie-local', 'up'], stdin=subprocess.DEVNULL).returncode:
        raise ValueError(name + ' did not start; see ~/.archie/local/' + name + '.log')
    models = manifest['files']
    spawn('core', ['archied', '--root', str(root), '--text-logs'])
    spawn('displayd', ['archie-displayd', '--root', str(root), '--orb-socket', str(state / 'orb.sock'),
                       '--disable-world-feed', '--text-logs'])
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        try:
            status = subprocess.run(['archie', '--root', str(root), 'status'],
                                    capture_output=True, timeout=2)
            if status.returncode == 0:
                with urllib.request.urlopen('http://127.0.0.1:7666/api/orb/lumen', timeout=2) as response:
                    if json.load(response).get('root_id'):
                        break
        except (OSError, ValueError, subprocess.TimeoutExpired):
            pass
        if not owned('core') or not owned('displayd'):
            raise ValueError('Core/displayd exited; inspect logs in ' + str(state))
        time.sleep(0.25)
    else:
        raise ValueError('Core/displayd did not become ready; inspect logs in ' + str(state))
    gui = {'XDG_RUNTIME_DIR': '/mnt/wslg/runtime-dir', 'WAYLAND_DISPLAY': 'wayland-0',
           'QT_QPA_PLATFORM': 'wayland', 'QT_QUICK_BACKEND': 'rhi', 'QSG_RHI_BACKEND': 'opengl',
           'QT_QUICK_CONTROLS_STYLE': 'Basic', 'LIBGL_ALWAYS_SOFTWARE': '1', 'GALLIUM_DRIVER': 'llvmpipe'}
    fresh = spawn('display', ['archie-display', '--shell', '--shell-bridge', '/usr/bin/archie-shell-bridge'], gui)
    if fresh:
        for _ in range(50):
            result = subprocess.run(['archie', 'visual', 'preset', 'polar-prisma'], capture_output=True)
            if result.returncode == 0:
                break
            time.sleep(0.2)
        else:
            raise ValueError('native display did not acknowledge its design; inspect ' + str(state / 'display.log'))
    if voice:
        spawn('voice', voice_arguments(profile, models, endpoint, alias),
              dict(gui, CUDA_VISIBLE_DEVICES='', OMP_NUM_THREADS='2', PIPER_THREADS='2',
                   PATH='/opt/archie/voice-venv/bin:/opt/archie/iris-venv/bin:' + os.environ['PATH'],
                   PULSE_SERVER='unix:/mnt/wslg/PulseServer'))
        time.sleep(1)
        if not owned('voice'):
            raise ValueError('voice exited; inspect ' + str(state / 'voice.log'))
    if profile['wanda']:
        wanda_dir = Path.home() / '.config/archie/wanda'
        if (wanda_dir / 'wanda.toml').is_file() and Path('/usr/bin/archie-wanda').exists():
            spawn('wanda', ['/usr/bin/archie-wanda', '--config', str(wanda_dir / 'wanda.toml'), '--listen', '127.0.0.1:8791'],
                  {'PWD': str(wanda_dir)})
        else:
            print('Wanda is on in the profile, but there is no ' + str(wanda_dir / 'wanda.toml') + ' (or no archie-wanda); it was not started.')
    print('Archie desktop started; say Archie. CPU responses can take several minutes. Logs: ' + str(state))

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='verify all included payload digests; start nothing')
    parser.add_argument('--prefix', type=Path, default=Path('/'), help='verification root; only valid with --check')
    parser.add_argument('--background', action='store_true', help='start the owned desktop session and return')
    parser.add_argument('--no-voice', action='store_true')
    args = parser.parse_args()
    if args.prefix != Path('/') and not args.check:
        parser.error('--prefix is only valid with --check')
    manifest = verify_bundle(args.prefix)
    if args.check:
        print(json.dumps({'integrity': 'ok', 'source_sha': manifest['source_sha'], 'files': len(manifest['files'])}))
    else:
        start(manifest, voice=not args.no_voice)

if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit('archie-offline: ' + str(error))
