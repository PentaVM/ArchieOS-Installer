"""External ToM benchmarks (evaluation only): FANToM and OpenToM on the native model.

Data and official code live outside git under /home/archie/wanda/bench (never in
datasets/, never trained on). Item construction and answer parsing follow the official
repositories: OpenToM imports its own prompt builder and evaluator; FANToM's setup and
scoring are ported line-for-line from eval_fantom.py (pandas/sklearn/evaluate are not
installed here, so their aggregations are reimplemented and unit-tested).
"""
from collections import Counter
import json
import os
import random
import sys
from pathlib import Path

BENCH = Path('/home/archie/wanda/bench')


# ---- shared metrics (sklearn-equivalent) ---------------------------------------------
def _per_label_f1(gold, pred, label):
    tp = sum(g == label and p == label for g, p in zip(gold, pred))
    fp = sum(g != label and p == label for g, p in zip(gold, pred))
    fn = sum(g == label and p != label for g, p in zip(gold, pred))
    return 0.0 if tp == 0 else 2*tp/(2*tp+fp+fn)


def f1_macro(gold, pred):
    labels = sorted(set(gold) | set(pred))
    return sum(_per_label_f1(gold, pred, l) for l in labels)/len(labels)


def f1_weighted(gold, pred):
    labels = sorted(set(gold) | set(pred))
    support = Counter(gold)
    total = sum(support.values())
    return sum(_per_label_f1(gold, pred, l)*support[l] for l in labels)/total


def accuracy(gold, pred):
    return sum(g == p for g, p in zip(gold, pred))/len(gold)


# ---- OpenToM -------------------------------------------------------------------------
OPENTOM_TYPES = ['location_cg_fo', 'location_cg_so', 'multihop_fo', 'multihop_so', 'attitude']


def opentom_items(seed=42, num_batch=5, batch_size=50):
    """Official run_baseline.py sampling (coarse location, normal-length narratives)."""
    import numpy as np
    src = BENCH/'src/opentom/src'
    sys.path.insert(0, str(src))
    from utils.build_prompt import build_prompt
    from utils.opentom_utils import OpenToMUtils
    data = src.parent/'data'/'opentom_data'
    load = lambda name: json.loads((data/name).read_text())
    prompt_dir = src/'prompts'/'chatgpt_opentom_prompts'
    template = lambda name: (prompt_dir/f'chatgpt_{name}.txt').read_text()
    model_info = {'chatgpt_prefix': [{'role': 'system', 'content': "You are an expert in modeling other's mental state."}],
                  'attitude_prompt_template': template('attitude'),
                  'location_cg_prompt_template': template('location_cg'),
                  'location_fg_prompt_template': template('location_fg'),
                  'multihop_fullness_prompt_template': template('multihop_fullness'),
                  'multihop_accessibility_prompt_template': template('multihop_accessibility'),
                  'preference_prompt_template': template('preference'),
                  'intention_prompt_template': template('intention')}
    random.seed(seed); np.random.seed(seed)
    meta = load('meta_data.json')
    questions = {t: load(f'{t}.json') for t in OPENTOM_TYPES}
    utils = OpenToMUtils()
    keys = list(meta.keys())
    random.shuffle(keys)
    batches = [keys[i:i+batch_size] for i in range(0, len(keys), batch_size)][:num_batch]
    items = []
    for b, key_list in enumerate(batches, 1):
        for key in key_list:
            val = meta[key]
            mover, affected, original, moved_to, eoi = utils.get_info(val)
            counter = 0
            for q_type in OPENTOM_TYPES:
                new = questions[q_type][key]
                if 'multihop' in q_type:
                    fullness = [i for i, q in enumerate(new) if 'fullness' in q['question']]
                    drop = np.random.choice(fullness, 2, replace=False)
                    new = [q for i, q in enumerate(new) if i not in drop]
                for q in new:
                    prompt, _ = build_prompt(q['question'], q['question'].replace("'s", '').split(), q, mover, affected,
                                             original, moved_to, eoi, val['narrative'], model_info, False, False, False,
                                             ac_preference=val['preferences']['observer'])
                    items.append({'id': f'b{b}:{key}:{counter}', 'batch': b, 'key': key, 'qid': str(counter),
                                  'question': q, 'messages': model_info['chatgpt_prefix'] + [{'role': 'user', 'content': prompt}]})
                    counter += 1
    return items


def opentom_score(items, predictions):
    """Official OpenToMEvaluator parsing + evaluate.py aggregation (mean over batches)."""
    src = BENCH/'src/opentom/src'
    sys.path.insert(0, str(src)); sys.path.insert(0, str(src/'evaluate'))
    from opentom_evaluator import OpenToMEvaluator
    result = {}
    for item in items:
        entry = dict(item['question'], prediction=predictions[item['id']])
        result.setdefault(f"batch-{item['batch']}", {}).setdefault(item['key'], {})[item['qid']] = entry
    path = BENCH/'runs'/'opentom-scoring-input.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(result))
    cwd = os.getcwd()
    try:
        os.chdir(src)                    # the evaluator opens ../data/opentom_data/meta_data.json
        per_type = OpenToMEvaluator().evaluate(str(path), 'coarse', 'all')
    finally:
        os.chdir(cwd)
    report = {}
    for q_type, batches in per_type.items():
        groups = {'': lambda kind: True} if not q_type.startswith('multihop') else {
            '_fullness': lambda kind: kind == 'fullness', '_accessibility': lambda kind: kind == 'accessibility',
            '_overall': lambda kind: True}
        for suffix, keep in groups.items():
            f1s, accs, corrupt = [], [], []
            for batch in batches:
                rows = [(g, p) for kind, g, p in batch if keep(kind)]
                if not rows:
                    continue
                valid = [(g, p) for g, p in rows if p != -1 and g is not None]
                corrupt.append(1 - len(valid)/len(rows))
                if valid:
                    gold, pred = zip(*valid)
                    f1s.append(f1_macro(gold, pred)); accs.append(accuracy(gold, pred))
            if f1s:
                import numpy as np
                report[q_type+suffix] = {'macro_f1': float(np.mean(f1s)), 'macro_f1_std': float(np.std(f1s)),
                                         'accuracy': float(np.mean(accs)), 'corrupted': float(np.mean(corrupt)),
                                         'batches': len(f1s)}
    return report


# ---- FANToM --------------------------------------------------------------------------
def fantom_items(seed=99, input_type='short'):
    """Port of FantomEvalAgent.setup_fantom (+ set_beliefQA_multiple_choices); same RNG order."""
    sets = json.loads((BENCH/'data/fantom/fantom_v1.json').read_text())
    rng = random.Random(seed)
    items = []

    def add(qa, text, set_id, **extra):
        items.append({'id': f'{len(items)}', 'set_id': set_id, 'qa': {**qa, **extra, 'set_id': set_id},
                      'messages': [{'role': 'user', 'content': text}]})

    for s in sets:
        context = (s['short_context'] if input_type == 'short' else s['full_context']).strip()
        set_id, fact_q, fact_a = s['set_id'], s['factQA']['question'], s['factQA']['correct_answer']
        add(s['factQA'], f"{context}\n\nQuestion: {fact_q}\nAnswer:", set_id)
        for qa in s['beliefQAs']:
            add(qa, f"{context}\n\nQuestion: {qa['question']}\nAnswer:", set_id)
            option_a, option_b = qa['wrong_answer'], qa['correct_answer']
            if rng.choice([True, False]):
                choices, answer = [option_a, option_b], 1
            else:
                choices, answer = [option_b, option_a], 0
            letters = ['(' + chr(x) + ')' for x in range(ord('a'), len(choices)+ord('a'))]
            choices_text = ''.join(f'{l} {o}\n' for l, o in zip(letters, choices))
            mc = f"{qa['question']}\n{choices_text.strip()}\n\nChoose an answer from above:"
            add(qa, f"{context}\n\nQuestion: {mc}", set_id, question=mc,
                question_type=qa['question_type']+':multiple-choice', correct_answer=answer)
        add(s['answerabilityQA_list'], f"{context}\n\nTarget: {fact_q}\nQuestion: {s['answerabilityQA_list']['question']}\nAnswer:", set_id)
        for qa in s['answerabilityQAs_binary']:
            add(qa, f"{context}\n\nTarget: {fact_q}\nQuestion: {qa['question']} Answer yes or no.\nAnswer:", set_id)
        add(s['infoAccessibilityQA_list'],
            f"{context}\n\nInformation: {fact_q} {fact_a}\nQuestion: {s['infoAccessibilityQA_list']['question']}\nAnswer:", set_id)
        for qa in s['infoAccessibilityQAs_binary']:
            add(qa, f"{context}\n\nInformation: {fact_q} {fact_a}\nQuestion: {qa['question']} Answer yes or no.\nAnswer:", set_id)
    return items


def _token_f1(truth, response):
    truth, response = truth.split(), response.split()
    same = sum((Counter(truth) & Counter(response)).values())
    if same == 0:
        return 0
    p, r = same/len(response), same/len(truth)
    return 2*p*r/(p+r)


def _mc(qa, response):
    answer = {0: 'a', 1: 'b', 2: 'c', 3: 'd'}[qa['correct_answer']]
    r = response.lower()
    return (r.startswith('('+answer+')') or r.startswith(answer+')') or r.startswith(answer+'.') or
            r.startswith(answer+':') or r.startswith(answer+',') or f'({answer})' in r or answer == r)


def _binary(response):
    a = response.lower().strip("'").strip('"')
    if (' yes,' in a or ' yes ' in a or a.startswith('yes') or ' yes.' in a or ' knows ' in a or a.startswith('true')):
        return 1
    if (' no,' in a or ' no ' in a or a.startswith('no') or ' no.' in a or ' does not know ' in a
            or " doesn't know " in a or a.startswith('false')):
        return 0
    return -1


YESNO = {'yes': 1, 'no': 0, 'no:long': 0, 'error': -1}


def parse_fantom_response(response):
    if 'Answer:' in response:
        return response.split('Answer:')[-1].strip()
    if 'Choose an answer from above:' in response:
        return response.split('Choose an answer from above:')[-1].strip()
    return response


def fantom_score(items, predictions, embed=None):
    """Port of evaluate_response + run_reports/score_and_analyze (set aggregation).

    embed: optional callable(list[str]) -> unit vectors for the free-form belief metric
    (official: sentence-transformers/all-roberta-large-v1). Without it, ALL* and
    belief:distance are reported as None, not estimated.
    """
    rows = []
    for item in items:
        qa, pred = item['qa'], parse_fantom_response(predictions[item['id']])
        t = qa['question_type']
        row = {'set_id': qa['set_id'], 'question_type': t, 'correct_answer': qa.get('correct_answer'),
               'missed_info_accessibility': qa.get('missed_info_accessibility'), 'result': None}
        if t.startswith('tom:belief:'):
            if t.endswith(':multiple-choice'):
                row['result'] = _mc(qa, pred)
            elif embed is not None:
                wrong, right, mine = embed([qa['wrong_answer'], qa['correct_answer'], pred])
                ok = float(mine @ wrong) < float(mine @ right)
                row['result'], row['word_overlap'] = ok, _token_f1(qa['correct_answer'] if ok else qa['wrong_answer'], pred)
        elif t.endswith(':list'):
            excluded = any(c.lower() not in pred.lower() for c in qa['correct_answer'])
            included = any(c.lower() in pred.lower() for c in qa['wrong_answer'])
            row['result'] = not (excluded or included)
        elif t.endswith(':binary'):
            row['binarized'] = _binary(pred)
            row['result'] = YESNO[qa['correct_answer']] == row['binarized']
        elif t.startswith('fact'):
            row['result'] = _token_f1(qa['correct_answer'].lower(), pred.lower())
        rows.append(row)
    rows = [r for r in rows if not (r['question_type'].endswith(':binary') and r['correct_answer'] == 'no:long')]
    report = {'fact_word-f1': _mean([r['result'] for r in rows if r['question_type'].startswith('fact')])}
    for scenario in ('inaccessible', 'accessible'):
        tom = [r for r in rows if r['question_type'].startswith('tom')]
        target = [r for r in tom if r['missed_info_accessibility'] == scenario]
        if scenario == 'accessible':
            sets = {s for s in {r['set_id'] for r in target}
                    if all(r['missed_info_accessibility'] == scenario for r in tom if r['set_id'] == s)}
        else:
            sets = {r['set_id'] for r in target}
        in_sets = [r for r in target if r['set_id'] in sets]
        free_form = [r for r in target if r['question_type'] == 'tom:belief:'+scenario]
        have_free_form = all(r['result'] is not None for r in free_form)
        all_types = {'tom:belief:'+scenario+':multiple-choice', 'tom:answerability:list', 'tom:answerability:binary',
                     'tom:info_accessibility:list', 'tom:info_accessibility:binary'}
        p = scenario+':'
        report[p+'set:ALL*'] = _all_by_set(in_sets) if have_free_form else None
        report[p+'set:ALL'] = _all_by_set([r for r in in_sets if r['question_type'] in all_types])
        report[p+'belief:multiple-choice'] = _mean([r['result'] for r in target if r['question_type'].endswith(':multiple-choice')])
        report[p+'belief:distance'] = _mean([r['result'] for r in free_form]) if have_free_form else None
        for family in ('answerability', 'info_accessibility'):
            fam = [r for r in target if r['question_type'].startswith('tom:'+family)]
            binary = [r for r in fam if r['question_type'].endswith(':binary')]
            report[p+family+':set:ALL'] = _all_by_set(fam)
            report[p+family+':list'] = _mean([r['result'] for r in fam if r['question_type'].endswith(':list')])
            report[p+family+':binary-f1'] = (f1_weighted([YESNO[r['correct_answer']] for r in binary],
                                                         [r['binarized'] for r in binary]) if binary else None)
    return report


def _mean(values):
    values = [v for v in values if v is not None]
    return sum(values)/len(values) if values else None


def _all_by_set(rows):
    by = {}
    for r in rows:
        by.setdefault(r['set_id'], []).append(bool(r['result']))
    return sum(all(v) for v in by.values())/len(by) if by else None
