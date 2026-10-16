"""Read-only run-root summary for supervisors; never imports Torch or touches the GPU."""
import json
import os
import time
from pathlib import Path


def _read(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def _alive(worker):
    if not worker:
        return False
    try:
        os.kill(worker['pid'], 0)
    except (OSError, KeyError, TypeError):
        return False
    boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    cmdline = Path(f'/proc/{worker["pid"]}/cmdline').read_bytes().replace(b'\0', b' ')
    return worker.get('boot_id') == boot and b'archie_wanda' in cmdline and b'worker' in cmdline


def summarize(root):
    root = Path(root)
    worker = _read(root/'worker.json')
    runs = {}
    for run in sorted((root/'runs').glob('*')):
        lines = (run/'metrics.jsonl').read_text().splitlines() if (run/'metrics.jsonl').exists() else []
        events = [json.loads(line) for line in lines]
        evals = [e for e in events if e['event'] in ('baseline', 'eval', 'final')]
        steps = [e for e in events if e['event'] == 'step']
        runs[run.name] = {'receipt': (run/'receipt.json').exists(), 'opt_step': steps[-1]['opt_step'] if steps else 0,
                          'last_train_loss': steps[-1]['train_loss'] if steps else None,
                          'baseline': evals[0] if evals else None, 'latest_eval': evals[-1] if evals else None}
    queue = {name: sorted(p.name for p in (root/'queue'/name if name else root/'queue').glob('*.json'))
             for name in ('', 'running', 'done', 'failed')}
    return {'now_ms': int(time.time()*1000), 'worker': worker, 'worker_alive': _alive(worker),
            'heartbeat_age_s': (time.time()*1000-worker['updated_ms'])/1000 if worker else None,
            'control': _read(root/'control.json'), 'queue': queue, 'runs': runs}
