#!/usr/bin/env python3
"""Explicit local desktop launch. No downloads, cloud fallback, or Stage authority."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import secrets
import socket
import subprocess
import time
import urllib.request

REQUIRED = {'bonsai', 'asr', 'piper_es', 'piper_en', 'iris'}

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

def identity(pid):
    try:
        # comm may contain spaces or parentheses; fields after its last ')' start at3.
        fields = Path('/proc', str(pid), 'stat').read_text().rsplit(')', 1)[1].split()
        return None if fields[0] == 'Z' else fields[19]
    except (OSError, IndexError):
        return None

def start(manifest, voice=True):
    import fcntl
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
    for port, name in [(8081, 'bonsai'), (7666, 'displayd'), (7668, 'display')]:
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
    model = root / 'etc/models/bonsai.toml'
    if not model.exists():
        model.write_text('''id = "local.bonsai2-9b"
display_name = "Bonsai2 9B local"
provider = "open-ai-compatible"
locality = "local"
context_window = 4096
max_output_tokens = 512
capabilities = ["cognition.response", "cognition.reasoning"]
max_privacy_grade = "restricted"
availability = "ready"
quality_tier = 0
latency_ema_ms = 0
[cost]
input_micros_per_mtok = 0
output_micros_per_mtok = 0
[labels]
provider-model = "bonsai"
base-url = "http://127.0.0.1:8081/v1"
''')
    models = manifest['files']
    # CPU is the conservative offline default; no CUDA process or training is started.
    spawn('bonsai', ['/opt/archie/llama-cuda/bin/llama-server', '--model', models['bonsai']['path'],
                    '--host', '127.0.0.1', '--port', '8081', '--alias', 'bonsai', '--jinja',
                    '--ctx-size', '4096', '--threads', '2', '--threads-batch', '2', '--parallel', '1',
                    '--n-gpu-layers', '0', '--device', 'none', '--predict', '160'],
          {'CUDA_VISIBLE_DEVICES': '', 'LD_LIBRARY_PATH': '/opt/archie/llama-cuda/lib'})
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
    for _ in range(60):
        try:
            with urllib.request.urlopen('http://127.0.0.1:8081/health', timeout=1) as response:
                if json.load(response).get('status') == 'ok':
                    break
        except (OSError, ValueError):
            time.sleep(1)
    else:
        raise ValueError('Bonsai did not become healthy; inspect ' + str(state / 'bonsai.log'))
    if voice:
        spawn('voice', ['archie-voice', 'converse', '--grade', 'internal', '--endpoint',
                       'http://127.0.0.1:8081/v1', '--model', 'bonsai', '--timeout-seconds', '600',
                       '--budget', 'sentence', '--language', 'es-CL', '--asr-model', models['asr']['path'],
                       '--voice-engine', 'piper', '--piper', '/usr/lib/archie/piper-json-bridge.py',
                       '--piper-model', models['piper_es']['path'], '--piper-model-en', models['piper_en']['path'],
                       '--threads', '2'],
              dict(gui, CUDA_VISIBLE_DEVICES='', OMP_NUM_THREADS='2', PIPER_THREADS='2',
                   PATH='/opt/archie/iris-venv/bin:' + os.environ['PATH'],
                   PULSE_SERVER='unix:/mnt/wslg/PulseServer'))
        time.sleep(1)
        if not owned('voice'):
            raise ValueError('voice exited; inspect ' + str(state / 'voice.log'))
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
