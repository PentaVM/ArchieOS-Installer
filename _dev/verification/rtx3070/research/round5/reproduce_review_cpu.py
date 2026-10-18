"""CPU-only review fixtures; never executes dashboard, recovery, WSL or subprocesses."""
import ast
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
from types import SimpleNamespace

REPO = Path(__file__).resolve().parents[5]
PKG = REPO / 'training/wanda_bonsai2'
sys.path.insert(0, str(PKG / 'src'))
from archie_wanda.contracts import digest
from archie_wanda.decision import build_envelope, build_correlation

spec = importlib.util.spec_from_file_location('review_decision_fixtures', PKG / 'tests/test_decision.py')
fixture_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture_module)
analysis = fixture_module._sample_analysis()
evidence = ['personal/ev-001', 'personal/ev-002']
envelope = build_envelope(analysis, decision_id='dec-review', task_id='task-review',
    domain='personal/owner', produced_at_ms=1700000000000, expires_at_ms=1700000100000,
    source_run_ref='run:review', objective_refs=['obj:trip-coordination'],
    evidence_refs=evidence, admitted_refs=evidence)
# The Python constructor inserts fields in the same sequence as Rust's declared structs.
# This reproduces JSON byte ordering, not an execution of the Rust verifier.
ordered_bytes = json.dumps(envelope, ensure_ascii=False, separators=(',', ':')).encode()
sorted_hash = digest(envelope)
ordered_hash = hashlib.sha256(ordered_bytes).hexdigest()
assert sorted_hash != ordered_hash
invented = copy.deepcopy(analysis)
invented['user_objectives'][0]['objective_ref'] = 'obj:model-invented'
accepted_invented = build_envelope(invented, decision_id='dec-review2', task_id='task-review',
    domain='personal/owner', produced_at_ms=1700000000000, expires_at_ms=1700000100000,
    source_run_ref='run:review', objective_refs=[], evidence_refs=evidence, admitted_refs=evidence)

dashboard = PKG / 'ops/dashboard.py'
raw = dashboard.read_bytes()
node = next(n for n in ast.parse(raw).body if isinstance(n, ast.FunctionDef) and n.name == 'get_adapters')
content = ''


class FixturePath:
    def __init__(self, path):
        self.path = str(path)
        self.name = Path(self.path).name

    def __truediv__(self, name):
        return FixturePath(self.path + '/' + name)

    def is_dir(self):
        return True

    def exists(self):
        return self.path.endswith('metrics.jsonl')

    def read_text(self, **kwargs):
        return content


scope = {'ROOT': Path('/fixture'), 'json': json,
         'glob': SimpleNamespace(glob=lambda pattern: ['fixture-run']),
         'pathlib': SimpleNamespace(Path=FixturePath)}
exec(compile(ast.Module(body=[node], type_ignores=[]), str(dashboard), 'exec'), scope)
content = json.dumps({'event': 'final', 'opt_step': 1, 'choice_accuracy': 0.90625,
    'accuracy_counterfactual': 0.625, 'probe_choice_accuracy': 0.25,
    'probe_accuracy_counterfactual': 0.0}) + '\n'
shown = scope['get_adapters']()[0]
assert shown['choice_acc'] == 0.90625 and shown['cf_acc'] == 0.625
content += '{"event":'
try:
    scope['get_adapters']()
    torn_exception = None
except json.JSONDecodeError:
    torn_exception = 'JSONDecodeError'
assert torn_exception == 'JSONDecodeError'
out = {'scope': 'pure Python/source fixtures; not hardware or model-quality evidence',
       'correlation': {'python_sorted_hash': sorted_hash, 'struct_order_hash_simulated': ordered_hash,
                       'python_correlation_hash': build_correlation(envelope)['decision_sha256'],
                       'rust_binary_executed': False},
       'empty_objectives': {'envelope_objectives': accepted_invented['objective_refs'],
                           'accepted_analysis_objective': invented['user_objectives'][0]['objective_ref']},
       'dashboard': {'source_sha256': hashlib.sha256(raw).hexdigest(), 'shown': shown,
                     'fixture_probe_choice': 0.25, 'fixture_probe_cf': 0.0,
                     'partial_tail_exception': torn_exception}}
print(json.dumps(out, indent=2))
