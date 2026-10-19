"""Read existing TRAIN candidates; native vocab-only token counts, no inference/jobs."""
import hashlib
import json
from pathlib import Path
import sys
import time
from types import SimpleNamespace

repo = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(repo / 'training/wanda_bonsai2/src'))
from archie_wanda.data import NativeTokenizer, build_examples, load_families, render

tokenizer = NativeTokenizer(SimpleNamespace(
    runtime_dir='/opt/archie/llama-cuda',
    model_path='/home/archie/wanda/models/Ternary-Bonsai-2-9B-PTQ1_0.gguf'))
start = time.monotonic()
result = {'scope': 'existing TRAIN candidates only; CPU vocab-only; no admission or training',
          'review': 'pending-human', 'seed': 17, 'candidates': []}
for relative in ['round2/curriculum-003-candidate.json', 'round3/curriculum-004-candidate.json']:
    path = Path(__file__).resolve().parents[1] / relative
    families = load_families(path)
    assert all(f['split'] == 'train' and f['provenance']['review'] == 'pending-human' for f in families)
    rows = []
    for example in build_examples(families, 17):
        if time.monotonic() - start > 180:
            raise RuntimeError('CPU audit time budget exhausted')
        prompt, full = render(example)
        rows.append({'id': example.example_id, 'family': example.family_id,
                     'domain': example.domain, 'full_tokens': len(tokenizer(full)),
                     'target_keys_before_choice': list(json.loads(example.target)).index('selected_option_id')})
    result['candidates'].append({'path': relative, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                                 'families': len(families), 'rows': rows})
result['elapsed_seconds'] = round(time.monotonic() - start, 3)
result['cuda_inference_jobs'] = 0
out = Path(__file__).with_name('data-readiness-20261019.json')
out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'receipt': str(out), 'families': sum(c['families'] for c in result['candidates']),
                  'variants': sum(len(c['rows']) for c in result['candidates']),
                  'max_tokens': max(r['full_tokens'] for c in result['candidates'] for r in c['rows']),
                  'elapsed_seconds': result['elapsed_seconds']}))
