"""Family-aware decision metrics shared by model scoring and trivial baselines.

A record is one scored variant: {family_id, variant, pred, selected, shown_to_original,
counterfactual_changes}. The family (not the variant) is the unit that matters:
family_all_correct requires every variant of a family to be right.
"""
import random
import re

VARIANTS = ('clean', 'noise', 'cosmetic', 'counterfactual')
OPTION = re.compile(r'^([A-D]): (.+)$')


def aggregate(records, prefix=''):
    scored = [r for r in records if r.get('pred') is not None]
    for r in scored:
        r['correct'] = r['pred'] == r['selected']
        r['original'] = dict(r['shown_to_original'])[r['pred']]
    n = max(1, len(scored))
    out = {prefix+'scored': len(scored), prefix+'choice_accuracy': sum(r['correct'] for r in scored)/n}
    for kind in VARIANTS:
        subset = [r for r in scored if r['variant'] == kind]
        out[prefix+'accuracy_'+kind] = sum(r['correct'] for r in subset)/max(1, len(subset))
    families = {}
    for r in scored:
        families.setdefault(r['family_id'], {})[r['variant']] = r
    complete = [f for f in families.values() if all(v in f for v in VARIANTS)]
    m = max(1, len(complete))
    out[prefix+'families_scored'] = len(complete)
    out[prefix+'family_all_correct'] = sum(all(f[v]['correct'] for v in VARIANTS) for f in complete)/m
    out[prefix+'noise_invariance'] = sum(f['noise']['original'] == f['clean']['original'] for f in complete)/m
    out[prefix+'cosmetic_invariance'] = sum(f['cosmetic']['original'] == f['clean']['original'] for f in complete)/m
    action = [f for f in complete if f['counterfactual']['counterfactual_changes'] == 'action']
    out[prefix+'counterfactual_flip_rate'] = (sum(f['counterfactual']['original'] != f['clean']['original']
                                                  for f in action)/max(1, len(action)))
    letters = [r['pred'] for r in scored]
    out[prefix+'max_letter_share'] = max((letters.count(x) for x in set(letters)), default=0)/n
    return out


def shown_options(user_text):
    """Option letter → action text exactly as shown in the prompt."""
    options = {}
    lines = user_text.splitlines()
    start = lines.index('Opciones:') + 1
    for line in lines[start:]:
        match = OPTION.match(line)
        if match:
            options[match.group(1)] = match.group(2)
    return options


def trivial_baselines(examples, seed=20261016):
    """Rules that read no situation: longest/shortest option, always A, uniform random."""
    rules = {
        'longest_option': lambda o, rng: max(sorted(o), key=lambda k: len(o[k])),
        'shortest_option': lambda o, rng: min(sorted(o), key=lambda k: len(o[k])),
        'always_A': lambda o, rng: 'A',
        'random': lambda o, rng: rng.choice(sorted(o)),
    }
    result = {}
    for name, rule in rules.items():
        rng = random.Random(seed)
        records = []
        for e in examples:
            options = shown_options(e['user'])
            records.append({'family_id': e['family_id'], 'variant': e['variant'], 'selected': e['selected'],
                            'shown_to_original': e['shown_to_original'],
                            'counterfactual_changes': e['counterfactual_changes'], 'pred': rule(options, rng)})
        result[name] = aggregate(records)
    return result
