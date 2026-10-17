"""CPU-only adversarial probes of the exact STaR acceptance functions.

Extracts their AST without importing the package, native runtime or tokenizer.
This is a data-admission audit, not a model evaluation or a hardware test.
"""
import argparse
import ast
import hashlib
import json
import re
from pathlib import Path


def audit(source):
    raw = source.read_bytes()
    tree = ast.parse(raw, filename=str(source))
    selected = {'normalise', 'answer_of', 'answers', 'accept', 'accept_rationale'}
    nodes = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name in selected]
    assert {node.name for node in nodes} == selected
    scope = {'re': re, 'LEAKS': re.compile(
        r'hint|pista|correct answer is|respuesta correcta|the answer is given|we are told the answer',
        re.IGNORECASE)}
    # Compile the LEAKS assignment itself so the audit follows later source revisions.
    assignments = [node for node in tree.body if isinstance(node, ast.Assign)
                   and any(isinstance(t, ast.Name) and t.id == 'LEAKS' for t in node.targets)]
    exec(compile(ast.Module(body=assignments + nodes, type_ignores=[]), str(source), 'exec'), scope)
    example = {'example_id': 'audit-only', 'target': 'no'}
    good = {'text': 'No. The observer missed the move.', 'thinking': 'The observer was absent.',
            'thinking_tokens': 12, 'forced_end': False}
    cases = [
        ('valid_bare_label', dict(good, text='no'), True),
        ('valid_label_explanation', good, True),
        ('longer_word', dict(good, text='Nobody knows.'), False),
        ('forced_end', dict(good, forced_end=True), False),
        ('over_cap', dict(good, thinking_tokens=841), False),
        ('negated_label', dict(good, text='No is incorrect; yes is the answer.'), False),
        ('explicit_self_correction', dict(good, text='No. Correction: yes.'), False),
        ('missing_completion_flag', {k: v for k, v in good.items() if k != 'forced_end'}, False),
        ('missing_token_count', {k: v for k, v in good.items() if k != 'thinking_tokens'}, False),
        ('negative_token_count', dict(good, thinking_tokens=-1), False),
        ('nan_token_count', dict(good, thinking_tokens=float('nan')), False),
    ]
    observations = []
    for name, result, desired in cases:
        actual = bool(scope['accept'](example, result, 840))
        observations.append({'case': name, 'accepted': actual, 'desired': desired,
                             'matches_desired': actual == desired})
    rationale = dict(good, thinking='The supplied solution says no; I will justify that conclusion.')
    observations.append({'case': 'unlisted_hint_paraphrase',
                         'accepted': bool(scope['accept_rationale'](example, rationale, 840)),
                         'interpretation': 'Lexical exclusion is not a certificate of causal independence.'})
    observations.append({'case': 'spanish_substring_false_positive',
                         'accepted': bool(scope['accept_rationale'](example, dict(good,
                             thinking='Ella estaba despistada y no vio el traslado.'), 840)),
                         'interpretation': 'The substring pista also occurs in the ordinary word despistada.'})
    native_source = source.with_name('native_chat.py')
    native_raw = native_source.read_bytes()
    native_tree = ast.parse(native_raw, filename=str(native_source))
    native_class = next(n for n in native_tree.body if isinstance(n, ast.ClassDef) and n.name == 'NativeChat')
    think = next(n for n in native_class.body if isinstance(n, ast.FunctionDef) and n.name == 'think')
    render = next(n for n in native_tree.body if isinstance(n, ast.FunctionDef) and n.name == 'render_chat')
    native_scope = {}
    exec(compile(ast.Module(body=[render, think], type_ignores=[]), str(native_source), 'exec'), native_scope)
    completions = iter([('The observer was absent.', 10, False), ('no', 1, True)])
    fake = type('PureCompletionFixture', (), {'think': native_scope['think'],
        '_complete': lambda self, *a, **kw: next(completions)})()
    output = fake.think([{'role': 'user', 'content': 'Synthetic admission audit only.'}], budget=1024)
    early_stop = {'completion_fixture': 'non-delimiter stop after 10 tokens, then correct answer',
                  'forced_end': output['forced_end'], 'accepted': bool(scope['accept'](example, output, 840)),
                  'native_source_sha256': hashlib.sha256(native_raw).hexdigest(),
                  'hardware_or_live_incidence_claim': False}
    return {'scope': 'pure-function CPU audit; synthetic adversarial inputs only',
            'source': str(source), 'source_sha256': hashlib.sha256(raw).hexdigest(),
            'functions': {node.name: {'start': node.lineno, 'end': node.end_lineno} for node in nodes},
            'cases': observations,
            'early_stop_fixture': early_stop,
            'adversarial_mismatches': sum(not o['matches_desired'] for o in observations if 'matches_desired' in o),
            'no_live_trace_incidence_estimated': True}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    report = audit(args.source)
    args.out.write_text(json.dumps(report, indent=2, allow_nan=False) + '\n', encoding='utf-8')
    print(json.dumps({'source_sha256': report['source_sha256'],
                      'cases': len(report['cases']), 'adversarial_mismatches': report['adversarial_mismatches']}))
