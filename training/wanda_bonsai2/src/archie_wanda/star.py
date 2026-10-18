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
    text = re.sub(r'\*+', '', text).strip().lower()
    text = re.sub(r'^(answer|respuesta|conclusion|conclusión|result|resultado)\s*:\s*', '', text)
    return re.sub(r'[\s.!]+$', '', text)


def answer_of(example):
    """The bare answer of a chat-v1 example (direct target, or the part after </think>)."""
    target = example['target']
    return target.split('\n</think>\n\n')[-1] if '\n</think>\n\n' in target else target


def _matches_label(candidate, label):
    candidate = normalise(candidate)
    labels = [label]
    if label not in ('the same', 'lo mismo'):
        bare = re.sub(r'^(the|el|la|los|las)\s+', '', label)
        if bare != label:
            labels.append(bare)
    matched = next((lb for lb in labels if candidate == lb or bool(re.match(re.escape(lb) + r'(?![\w-])', candidate))), None)
    if matched is None:
        return False
    tail = candidate[len(matched):]
    if re.search(r'\b(is\s+(incorrect|wrong|false)|es\s+(incorrect[oa]|fals[oa]))\b', tail):
        return False
    opposites = {
        'yes': ('no',), 'sí': ('no',), 'si': ('no',),
        'no': ('yes', 'sí', 'si'),
        'more': ('less', 'the same'), 'more full': ('less full', 'the same'),
        'less': ('more', 'the same'), 'less full': ('more full', 'the same'),
        'the same': ('more', 'less', 'more full', 'less full'),
        'más': ('menos', 'lo mismo'), 'más accesible': ('menos accesible', 'igual de accesible'),
        'menos': ('más', 'lo mismo'), 'menos accesible': ('más accesible', 'igual de accesible'),
        'lo mismo': ('más', 'menos'), 'igual de accesible': ('más accesible', 'menos accesible'),
        'positive': ('negative', 'neutral'), 'negative': ('positive', 'neutral'), 'neutral': ('positive', 'negative'),
        'positivo': ('negativo', 'neutral'), 'negativo': ('positivo', 'neutral'),
        'positiva': ('negativa', 'neutral'), 'negativa': ('positiva', 'neutral'),
        'firm': ('tentative', 'declining'), 'tentative': ('firm', 'declining'), 'declining': ('firm', 'tentative'),
        'firme': ('tentativo', 'rechazando'), 'tentativo': ('firme', 'rechazando'), 'rechazando': ('firme', 'tentativo'),
        'pressured': ('supported', 'unbothered'), 'supported': ('pressured', 'unbothered'), 'unbothered': ('pressured', 'supported'),
        'presionado': ('apoyado', 'tranquilo'), 'apoyado': ('presionado', 'tranquilo'), 'tranquilo': ('presionado', 'apoyado'),
    }.get(label, ())
    if any(re.search(r'(?<!\w)' + re.escape(opp) + r'(?!\w)', tail) for opp in opposites):
        return False
    return True


def answers(text, label):
    """The answer reaches the label if it is the label, opens with it ("Yes. Hugo saw..."),
    or concludes with a final Answer/Respuesta line after brief reasoning, without
    contradicting itself with the opposite yes/no label. The training target keeps only
    the bare label."""
    label = normalise(label)
    if _matches_label(text, label):
        return True
    lines = [ln.strip() for ln in text.strip().splitlines() if ln.strip()]
    if len(lines) > 1 and _matches_label(lines[-1], label):
        return True
    return False


def accept(example, result, max_thinking_tokens):
    """Keep a generated trace iff it ended on its own, fits, and reaches the simulated label."""
    tokens = result.get('thinking_tokens', 0)
    if result.get('forced_end') is not False or type(tokens) is not int or not 0 < tokens <= max_thinking_tokens:
        return False
    if not result.get('thinking', '').strip():
        return False
    return answers(result.get('text', ''), answer_of(example))


HINTS = {'en': '\n\n(Hint: the correct answer is "{}". Reason step by step to it.)',
         'es': '\n\n(Pista: la respuesta correcta es "{}". Razona paso a paso hasta ella.)'}
LEAKS = re.compile(r'\bhint\b|\bpista\b|correct answer is|respuesta correcta|the answer is given|we are told the answer|supplied solution',
                   re.IGNORECASE)


def hinted(example):
    """STaR rationalization prompt: the same conversation with the simulated label as a hint.
    Only the trace is kept; training uses the original, hint-free messages."""
    messages = [dict(m) for m in example['messages']]
    messages[-1]['content'] += HINTS.get(example.get('lang', 'en'), HINTS['en']).format(answer_of(example))
    return messages


def accept_rationale(example, result, max_thinking_tokens):
    """A rationalization is kept like a trace, and only if it never refers to the hint."""
    return accept(example, result, max_thinking_tokens) and not LEAKS.search(result.get('thinking', ''))


def build_tranche(source, results, out, max_thinking_tokens, rationales=None):
    """New chat-v1 tranche with accepted self-generated traces as think-mode targets; where the
    unhinted attempt failed, an accepted hint-free-looking rationalization is used instead."""
    data = load_tranche(source)
    rationales = rationales or {}
    kept, counts = [], {'self': 0, 'rationalized': 0}
    for example in data['examples']:
        result, source_kind = results.get(example['example_id']), 'self'
        if result is None or not accept(example, result, max_thinking_tokens):
            result, source_kind = rationales.get(example['example_id']), 'rationalized'
            if result is None or not accept_rationale(example, result, max_thinking_tokens):
                continue
        counts[source_kind] += 1
        kept.append(dict(example, mode='think', trace=source_kind,
                         target=result['thinking'].strip() + '\n</think>\n\n' + answer_of(example)))
    provenance = dict(data['provenance'], kind='owned-procedural', traces='self-generated, label-filtered',
                      source=str(source), accepted=len(kept), attempted=len(results), **counts)
    Path(out).write_text(json.dumps({'format': FORMAT, 'provenance': provenance, 'examples': kept},
                                    ensure_ascii=False, indent=0), encoding='utf-8')
    return {'attempted': len(results), 'accepted': len(kept), **counts}
