import dataclasses
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from archie_wanda import admission
from archie_wanda.contracts import AdmissionError, TrainConfig, BASE_SHA, PRISM_COMMIT
from archie_wanda.native import STRICT_F32_ENV, sha_file


class TrainingAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.config = TrainConfig(model_path='/unused', runtime_dir=str(self.root))
        for name in ('libllama.so', 'libggml-cuda.so', 'libggml-base.so'):
            (self.root / name).write_bytes(name.encode())
        self.extraction = self.write('extraction.json', {
            'base_sha256': BASE_SHA, 'runtime_commit': PRISM_COMMIT,
            'scope': 'real_native_full_row_capture_microbatch_reset', 'shim_sha256': 'a' * 64,
            'precision_profile': 'strict-f32', 'precision_environment': STRICT_F32_ENV,
            'cases': [{'id': name, 'tokens': 10, 'comparisons': {
                kind: {**{file: {'relative_rms': .0001} for file in ('prefix.f32','norm.f32','logits.f32')},
                       'distribution': {'max_token_kl': .00001}}
                for kind in ('fresh_reset','microbatch')}} for name in ('a','b','c')],
            'runtime_library_sha256': {p.name: sha_file(p) for p in self.root.glob('*.so')}})
        self.parity = self.write('parity.json', {
            'status': 'passed', 'scope': 'real_zero_adapter_suffix',
            'base_sha256': self.config.base_sha256, 'precision_profile': 'strict-f32',
            'extraction_sha256': sha_file(self.extraction),
            'torch_float32_matmul_precision': 'highest',
            'cases': [{'id': name, 'tokens': 10, 'criterion': 'strict', 'normalized_relative_rms': .0001,
                       'maximum_token_kl': .00001} for name in ('a','b','c')]})
        self.backward = self.write('backward.json', {
            'status': 'passed', 'scope': 'real_suffix_full_vocabulary_backward',
            'base_sha256': self.config.base_sha256, 'precision_profile': 'strict-f32',
            'torch_float32_matmul_precision': 'highest', 'tokens': 10, 'vocabulary': 248320,
            'steps': [{'step': step, 'loss': 2., 'gradient_norms': {
                name: .1 for name in ('q_delta.a','q_delta.b','v_delta.a','v_delta.b')}} for step in (1,2)],
            'parity_sha256': sha_file(self.parity)})
        self.spec = {'precision_profile': 'strict-f32', 'extraction_receipt': str(self.extraction),
                     'parity_receipt': str(self.parity), 'backward_receipt': str(self.backward)}

    def tearDown(self):
        self.temp.cleanup()

    def write(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value))
        return path

    def test_strict_chain_and_unchanged_runtime_are_required(self):
        self.assertEqual(admission.validate_gates(self.config, self.spec)['precision_profile'], 'strict-f32')
        (self.root / 'libggml-cuda.so').write_bytes(b'changed runtime')
        with self.assertRaises(AdmissionError):
            admission.validate_gates(self.config, self.spec)

    def test_legacy_or_missing_gate_cannot_start_training(self):
        with self.assertRaises(AdmissionError):
            admission.validate_gates(self.config, {})
        data = json.loads(self.parity.read_text())
        data['cases'][0]['criterion'] = 'within_native_layout_noise'
        self.write('parity.json', data)
        with self.assertRaises(AdmissionError):
            admission.validate_gates(self.config, self.spec)

    def test_run_identity_changes_when_labels_or_configuration_change(self):
        examples = [{'example_id': 'example', 'cache': '/same', 'selected': 'A'}]
        initial = admission.run_identity(self.config, {}, examples, [], {})
        examples[0]['selected'] = 'B'
        self.assertNotEqual(initial, admission.run_identity(self.config, {}, examples, [], {}))
        altered = dataclasses.replace(self.config, sequence_length=64)
        self.assertNotEqual(initial, admission.run_identity(altered, {}, examples, [], {}))

    def test_lora_pairs_and_cache_precision_cannot_be_partially_restored(self):
        with self.assertRaises(AdmissionError):
            admission.validate_lora_keys({'q_delta.a': object()}, ['q'])
        admission.validate_lora_keys({'q_delta.a': object(), 'q_delta.b': object()}, ['q'])
        with self.assertRaises(AdmissionError):
            admission.validate_cache_profile({'precision_profile': 'native'}, {'precision_profile': 'strict-f32'})

    def test_backward_needs_two_finite_full_vocab_steps_with_complete_qv_gradients(self):
        original = json.loads(self.backward.read_text())
        for change in ('empty', 'nan', 'missing_gradient', 'zero_second_gradient', 'wrong_vocab', 'medium'):
            value = json.loads(json.dumps(original))
            if change == 'empty': value['steps'] = []
            if change == 'nan': value['steps'][0]['loss'] = float('nan')
            if change == 'missing_gradient': del value['steps'][1]['gradient_norms']['q_delta.a']
            if change == 'zero_second_gradient': value['steps'][1]['gradient_norms']['v_delta.a'] = 0.
            if change == 'wrong_vocab': value['vocabulary'] = 100
            if change == 'medium': value['torch_float32_matmul_precision'] = 'medium'
            self.write('backward.json', value)
            with self.subTest(change=change), self.assertRaises(AdmissionError):
                admission.validate_gates(self.config, self.spec)

    def test_extraction_needs_three_finite_cases_and_qv_proof_does_not_admit_other_targets(self):
        value = json.loads(self.extraction.read_text())
        value['cases'] = []
        self.write('extraction.json', value)
        parity = json.loads(self.parity.read_text())
        parity['extraction_sha256'] = sha_file(self.extraction)
        self.write('parity.json', parity)
        backward = json.loads(self.backward.read_text())
        backward['parity_sha256'] = sha_file(self.parity)
        self.write('backward.json', backward)
        with self.assertRaises(AdmissionError):
            admission.validate_gates(self.config, self.spec)
        with self.assertRaises(AdmissionError):
            admission.validate_gates(self.config, dict(self.spec, targets=['q','k','v','o']))


class TrainingPrecisionTests(unittest.TestCase):
    def test_precision_policy_rejects_overrides_and_records_effective_highest(self):
        self.assertTrue(hasattr(admission, 'configure_training_precision'))
        fake = SimpleNamespace()
        fake.policy = 'medium'
        fake.set_float32_matmul_precision = lambda value: setattr(fake, 'policy', value)
        fake.get_float32_matmul_precision = lambda: fake.policy
        fake.backends = SimpleNamespace(cuda=SimpleNamespace(matmul=SimpleNamespace(allow_tf32=True)))
        with patch.dict('os.environ', {'TORCH_ALLOW_TF32_CUBLAS_OVERRIDE': '1'}):
            with self.assertRaises(AdmissionError): admission.configure_training_precision(fake)
        with patch.dict('os.environ', {'TORCH_ALLOW_TF32_CUBLAS_OVERRIDE': '0', 'NVIDIA_TF32_OVERRIDE': '0'}):
            policy = admission.configure_training_precision(fake)
            self.assertEqual(policy['float32_matmul_precision'], 'highest')
            self.assertFalse(policy['allow_tf32'])

    def test_actual_worker_entry_pins_precision_before_any_training_allocation(self):
        from archie_wanda.worker import Worker
        class ReachedPolicy(Exception): pass
        with tempfile.TemporaryDirectory() as directory:
            worker = Worker(TrainConfig(model_path='/unused'), directory, 'headers', 0)
            with patch('archie_wanda.admission.validate_gates', return_value={}), \
                 patch('archie_wanda.admission.configure_training_precision', side_effect=ReachedPolicy,
                       create=True) as policy:
                with self.assertRaises(ReachedPolicy):
                    worker.train({'run_id': 'cpu-spy', 'datasets': []})
                policy.assert_called_once()
