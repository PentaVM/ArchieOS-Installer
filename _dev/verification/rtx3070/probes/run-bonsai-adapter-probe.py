"""Start only the unpromoted adapter compatibility probe, authenticated on loopback."""
import os
from pathlib import Path
import secrets

state = Path.home() / '.local/state/archie-bonsai-compat'
state.mkdir(parents=True, exist_ok=True, mode=0o700)
key = state / 'api.key'
if not key.exists():
    descriptor = os.open(key, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, 'w') as stream:
        stream.write(secrets.token_hex(32))
binary = '/opt/archie/llama-cuda/llama-server'
args = [binary, '-m', '/mnt/d/ArchieLocal/models/Ternary-Bonsai-2-9B-PTQ1_0.gguf',
        '--host', '127.0.0.1', '--port', '18081', '-ngl', '99', '-c', '4096', '-b', '64', '-ub', '32',
        '--parallel', '1', '--threads', '4', '--jinja', '--no-warmup', '--api-key-file', str(key),
        '--lora', '/mnt/d/ArchieLocal/compatibility-probes/bonsai9b-zero-lora-PROBE.gguf,/mnt/d/ArchieLocal/compatibility-probes/bonsai9b-one-layer-lora-PROBE.gguf',
        '--lora-init-without-apply']
os.execve(binary, args, dict(os.environ, LD_LIBRARY_PATH='/opt/archie/llama-cuda/lib:/usr/lib/wsl/lib', OMP_NUM_THREADS='4'))
