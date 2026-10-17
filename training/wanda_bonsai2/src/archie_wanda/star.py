"""Self-generated reasoning traces filtered by simulated labels (STaR / s1-style curation).

The model reasons on owned procedural problems with budget forcing; a trace is kept only
when it ended naturally (not cut by the budget) and its final answer equals the label the
simulation computed. Kept traces become think-mode chat-v1 targets in the model's own
reasoning style, so training does not overwrite it with short templated traces.
"""
import json
import re
from pathlib import Path
from .chatdata import FORMAT, load_tranche


def normalise(text):
    text = text.strip().lower()
    text = re.sub(r'^(answer|respuesta)\s*:\s*', '', text)
    return re.sub(r'[\s.!]+$', '', text)


def answer_of(example):
    """The bare answer of a chat-v1 example (direct target, or the part after </think>)."""
    target = example['target']
    return target.split('\n</think>\n\n')[-1] if '\n</think>\n\n' in target else target


def accept(example, result, max_thinking_tokens):
    """Keep a generated trace iff it ended on its own, fits, and reaches the simulated label."""
    if result.get('forced_end') or result.get('thinking_tokens', 0) > max_thinking_tokens:
        return False
    if not result.get('thinking', '').strip():
        return False
    return normalise(result['text']) == normalise(answer_of(example))


def build_tranche(source, results, out, max_thinking_tokens):
    """New chat-v1 tranche with the accepted self-generated traces as think-mode targets."""
    data = load_tranche(source)
    kept = []
    for example in data['examples']:
        result = results.get(example['example_id'])
        if result is None or not accept(example, result, max_thinking_tokens):
            continue
        kept.append(dict(example, mode='think',
                         target=result['thinking'].strip() + '\n</think>\n\n' + answer_of(example)))
    provenance = dict(data['provenance'], kind='owned-procedural', traces='self-generated, label-filtered',
                      source=str(source), accepted=len(kept), attempted=len(results))
    Path(out).write_text(json.dumps({'format': FORMAT, 'provenance': provenance, 'examples': kept},
                                    ensure_ascii=False, indent=0), encoding='utf-8')
    return {'attempted': len(results), 'accepted': len(kept)}
