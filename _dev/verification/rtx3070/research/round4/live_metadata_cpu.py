"""Read a bounded snapshot of worker/STaR metadata without importing Wanda.

Run with the system Python in WSL. Writes nothing there and emits no question,
answer, trace, credential, evaluation item or prompt content.
"""
import json
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path


root = Path('/home/archie/wanda/run')
out = {'read_at_utc': datetime.now(timezone.utc).isoformat(), 'read_only': True}
state = root / 'worker.json'
if state.exists() and state.stat().st_size <= 65536:
    data = json.loads(state.read_text())
    out['worker'] = {k: data[k] for k in ('state', 'pid', 'job', 'progress', 'updated_ms', 'updated_at') if k in data}
out['star_files'] = []
files = list((root / 'star').glob('*/*.jsonl')) if (root / 'star').exists() else []
for file in sorted(files, key=lambda p: p.stat().st_mtime, reverse=True)[:8]:
    size = file.stat().st_size
    if size > 32 * 1024 * 1024:
        out['star_files'].append({'path': str(file), 'size': size, 'skipped': 'over read budget'})
        continue
    counts, tokens = Counter(), []
    incomplete = False
    for line in file.read_text().splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            incomplete = True
            continue
        counts['records'] += 1
        counts['forced_end_true'] += record.get('forced_end') is True
        counts['missing_forced_end'] += 'forced_end' not in record
        counts['missing_thinking_tokens'] += 'thinking_tokens' not in record
        count = record.get('thinking_tokens')
        if type(count) is int and count >= 0:
            tokens.append(count)
    out['star_files'].append({'path': str(file), 'size': size,
        'mtime_utc': datetime.fromtimestamp(file.stat().st_mtime, timezone.utc).isoformat(),
        'counts': dict(counts), 'partial_line_seen': incomplete,
        'mean_thinking_tokens': sum(tokens) / len(tokens) if tokens else None})
print(json.dumps(out, indent=2))
