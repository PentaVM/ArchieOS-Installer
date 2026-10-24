"""chat-v1 tranches: system/user prompts with a <think> trace + answer target.

Used for procedurally generated problems (tomgen). Splits are by world, never by
question, so related questions about one story stay on one side.
"""
import hashlib
import json
from pathlib import Path
from .contracts import AdmissionError
from .native_chat import render_chat

FORMAT = 'chat-v1'


def world_split(world_id, development_share=0.1):
    bucket = int(hashlib.sha256(world_id.encode()).hexdigest()[:8], 16) / 0xFFFFFFFF
    return 'development' if bucket < development_share else 'train'


def write_tranche(problems, path, provenance, development_share=0.1):
    examples = []
    for n, p in enumerate(problems):
        examples.append({'example_id': f'{p.world_id}:{n}', 'family_id': p.world_id,
                         'split': world_split(p.world_id, development_share), 'variant': p.kind, 'lang': p.lang,
                         'domain': 'personal', 'messages': p.messages(), 'target': p.target(), 'label': p.label,
                         'mode': p.mode, 'pair': p.meta.get('pair'), 'side': p.meta.get('side')})
    Path(path).write_text(json.dumps({'format': FORMAT, 'provenance': provenance, 'examples': examples},
                                     ensure_ascii=False, indent=0), encoding='utf-8')
    return examples


def load_tranche(path):
    data = json.loads(Path(path).read_text(encoding='utf-8'))
    if data.get('format') != FORMAT or not data.get('examples'):
        raise AdmissionError('not a chat-v1 tranche')
    if data.get('provenance', {}).get('kind') not in ('owned-synthetic', 'owned-procedural'):
        raise AdmissionError('chat tranche needs owned provenance')
    ids = [e['example_id'] for e in data['examples']]
    if len(ids) != len(set(ids)):
        raise AdmissionError('duplicate example id')
    families = {}
    for e in data['examples']:
        if families.setdefault(e['family_id'], e['split']) != e['split']:
            raise AdmissionError('a world crosses train/development')
    return data


def render(example):
    """(prompt, full). think mode: the prompt ends inside the assistant's opened <think>
    block. direct mode: the prompt carries an empty think block (the base template's
    non-thinking form) and the target is the bare answer."""
    if example.get('mode', 'think') == 'direct':
        prompt = render_chat(example['messages']) + '<think>\n\n</think>\n\n'
    else:
        prompt = render_chat(example['messages']) + '<think>\n'
    return prompt, prompt + example['target'] + '<|im_end|>'
