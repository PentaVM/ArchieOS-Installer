"""Bounded read-only metadata/aggregate review; no Wanda imports or model loads."""
import json
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path('/home/archie/wanda/run')


def read_json(path):
    if not path.exists():
        return {'absent': True}
    if path.stat().st_size > 2 * 1024 * 1024:
        return {'skipped': 'over bounded metadata size'}
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError) as error:
        return {'read_error': type(error).__name__}


out = {'read_at_utc': datetime.now(timezone.utc).isoformat(), 'read_only': True}
out['g3_star'] = read_json(ROOT / 'star/g3-base-640/summary.json')
out['runs'] = {}
metric_keys = ('opt_step', 'opt_steps', 'train_loss', 'dev_loss', 'scored', 'choice_accuracy',
               'accuracy_counterfactual', 'probe_choice_accuracy', 'probe_accuracy_counterfactual',
               'probe_counterfactual_flip_rate', 'elapsed_seconds', 'stop_reason', 'run_digest',
               'base_sha256', 'train_examples', 'dev_examples', 'method', 'data_review')
for name in ('g2-direct-qkvo-s17', 'g3-star-qkvo-s17', 'g4-behav-star-qkvo-s17'):
    path = ROOT / 'runs' / name
    receipt = read_json(path / 'receipt.json')
    spec = read_json(path / 'spec.json')
    short_receipt = {k: v for k, v in receipt.items() if k in metric_keys or k == 'absent'}
    for phase in ('baseline', 'final', 'best'):
        if phase in receipt:
            short_receipt[phase] = {k: v for k, v in receipt[phase].items() if k in metric_keys}
    record = {'receipt': short_receipt, 'settings': spec.get('settings'),
              'train_examples': spec.get('train_examples'), 'dev_examples': spec.get('dev_examples'),
              'checkpoint_files': {f.name: {'bytes': f.stat().st_size, 'mtime': f.stat().st_mtime}
                                   for f in path.glob('*.pt')}}
    metric_path = path / 'metrics.jsonl'
    if metric_path.exists() and metric_path.stat().st_size < 16 * 1024 * 1024:
        last = {}
        for line in metric_path.read_text().splitlines():
            try:
                item = json.loads(line)
                last[item.get('event', '?')] = {k: v for k, v in item.items() if k in metric_keys}
            except ValueError:
                record['partial_metric_line'] = True
        record['last_events'] = last
    out['runs'][name] = record
out['benchmarks'] = {}
for name in ('opentom-base', 'opentom-g2-think512-pilot', 'opentom-g3-think512-pilot',
             'longchat-24k-pack2-direct', 'longchat-24k-pack2-v2', 'fantom-base'):
    data = read_json(ROOT / 'bench' / name / 'report.json')
    out['benchmarks'][name] = {k: v for k, v in data.items() if k in (
        'absent', 'read_error', 'skipped', 'benchmark', 'bench_id', 'adapter', 'base_sha256',
        'items', 'max_tokens', 'mode', 'thinking', 'budget', 'filters', 'forced_end',
        'mean_thinking_tokens', 'truncated', 'report', 'created_ms', 'protocol')}
    report = out['benchmarks'][name].get('report', {})
    if data.get('benchmark') == 'opentom':
        out['benchmarks'][name]['report'] = {k: {m: v for m, v in entry.items()
            if m in ('macro_f1', 'accuracy', 'corrupted')} for k, entry in report.items()
            if k in ('location-fo', 'location-so', 'multihop-fo_overall', 'multihop-so_overall', 'attitude')}
print(json.dumps(out, indent=2))
