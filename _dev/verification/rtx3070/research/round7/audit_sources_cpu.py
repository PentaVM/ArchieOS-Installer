"""Bounded public-source audit; never imports training/model code or writes a queue."""
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import urlopen


def fetch(url, cap=8 * 1024 * 1024):
    with urlopen(url, timeout=30) as response:
        raw = response.read(cap + 1)
    if len(raw) > cap:
        raise RuntimeError('Source exceeded the CPU audit download bound')
    return raw


def norm(value):
    return ' '.join(value.casefold().split())


repos = ['samhog/psychology-10k',
         'Gopher-Lab/huberman_lab_Dr__Becky_Kennedy_Protocols_for_Excellent_Parenting__Improving_Relationships_of_All']
receipt = {'scope': 'public metadata and bounded content audit only; no training or queue edits',
           'sources': [], 'rows_admitted': 0, 'gpu_jobs_started': 0}
for repo in repos:
    metadata = json.loads(fetch('https://huggingface.co/api/datasets/' + repo))
    revision = metadata['sha']
    card = metadata.get('cardData') or {}
    item = {'repo': repo, 'revision': revision, 'url': 'https://huggingface.co/datasets/' + repo,
            'declared_license': card.get('license'), 'card_metadata_present': bool(card),
            'files': [f['rfilename'] for f in metadata['siblings']]}
    if repo == repos[0]:
        raw = fetch(item['url'] + '/resolve/' + revision + '/Psychology-10K.json')
        rows = json.loads(raw)
        assert isinstance(rows, list)
        inputs, outputs, pairs, instructions = Counter(), Counter(), Counter(), Counter()
        by_input = defaultdict(set)
        for row in rows:
            a, b = norm(row['input']), norm(row['output'])
            inputs[a] += 1
            outputs[b] += 1
            pairs[(a, b)] += 1
            by_input[a].add(b)
            instructions[row.get('instruction', '')] += 1
        item.update({'file_sha256': hashlib.sha256(raw).hexdigest(), 'download_bytes': len(raw),
                     'rows': len(rows), 'fields': sorted(set().union(*(set(r) for r in rows))),
                     'unique_normalized_inputs': len(inputs), 'unique_normalized_outputs': len(outputs),
                     'duplicate_normalized_pairs_beyond_first': len(rows) - len(pairs),
                     'repeated_input_rows_beyond_first': len(rows) - len(inputs),
                     'inputs_with_multiple_distinct_outputs': sum(len(v) > 1 for v in by_input.values()),
                     'instruction_classes': len(instructions),
                     'rows_with_professional_role_condition': sum(n for text, n in instructions.items() if 'licensed psychologist' in text.lower()),
                     'max_input_chars': max(len(r['input']) for r in rows),
                     'max_output_chars': max(len(r['output']) for r in rows)})
    else:
        rows, response_hashes, truncated = [], [], 0
        for offset in (0, 100):
            url = 'https://datasets-server.huggingface.co/rows?' + urlencode({
                'dataset': repo, 'config': 'default', 'split': 'train', 'offset': offset, 'length': 100})
            raw = fetch(url, 2 * 1024 * 1024)
            response_hashes.append(hashlib.sha256(raw).hexdigest())
            data = json.loads(raw)
            for wrapped in data['rows']:
                rows.append(wrapped['row'])
                truncated += bool(wrapped.get('truncated_cells'))
        assert len(rows) == card['dataset_info']['splits'][0]['num_examples']
        after = json.loads(fetch('https://huggingface.co/api/datasets/' + repo))
        assert after['sha'] == revision, 'Source changed during audit'
        item.update({'rows': len(rows), 'fields': sorted(rows[0]),
                     'speaker_labels': dict(Counter(str(r['speaker']) for r in rows)),
                     'duplicate_normalized_text_rows_beyond_first': len(rows) - len({norm(r['text']) for r in rows}),
                     'total_text_chars': sum(len(r['text']) for r in rows),
                     'max_segment_chars': max(len(r['text']) for r in rows),
                     'truncated_rows_in_viewer': truncated, 'viewer_response_sha256': response_hashes,
                     'grouping': 'one named podcast episode; speaker segments are not independent train/dev families',
                     'pinning_note': 'README pinned; viewer responses hashed; repo revision unchanged before/after viewer audit'})
    item['direct_training_admission'] = 'not admitted: no decision_v1 causal-family supervision or verified license/provenance dossier'
    receipt['sources'].append(item)
out = Path(__file__).with_name('source-audit-20261019.json')
out.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(json.dumps(receipt, ensure_ascii=True))
