import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import numpy as np
from archie_wanda.contracts import AdmissionError, ParityError
from archie_wanda.native import judge_native_comparisons, validate_cache


class NativeCacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        np.ones((3, 4), dtype='<f4').tofile(self.root / 'prefix.f32')
        self.receipt = {'format_version': 1, 'base_sha256': 'a'*64, 'split': 'train',
                        'tokens': [1, 2, 3], 'positions': [0, 1, 2], 'sequence_ids': [0, 0, 0],
                        'control_vectors': 'none', 'capture_layer': 30, 'hidden_size': 4,
                        'files': {'prefix.f32': {'shape': [3, 4], 'dtype': 'float32',
                                                 'sha256': hashlib.sha256((self.root/'prefix.f32').read_bytes()).hexdigest()}}}

    def tearDown(self):
        assert self.root.resolve().parent == Path(tempfile.gettempdir()).resolve()
        self.temp.cleanup()

    def check(self, cap=1024):
        (self.root/'receipt.json').write_text(json.dumps(self.receipt))
        return validate_cache(self.root, 'a'*64, 'train', [1, 2, 3], cap)

    def test_full_rows_are_admitted(self):
        self.assertEqual(self.check()['files']['prefix.f32']['shape'], [3, 4])

    def test_last_row_only_does_not_count_as_full_capture(self):
        self.receipt['files']['prefix.f32']['shape'] = [1, 4]
        with self.assertRaises(AdmissionError): self.check()

    def test_wrong_split_positions_state_or_control_vectors_refuse(self):
        for field, value in [('split', 'sealed'), ('positions', [7, 8, 9]),
                             ('sequence_ids', [0, 0, 1]), ('control_vectors', 'unreviewed')]:
            original = self.receipt[field]; self.receipt[field] = value
            with self.subTest(field=field), self.assertRaises(AdmissionError): self.check()
            self.receipt[field] = original

    def test_cache_size_digest_and_path_escape_refuse(self):
        with self.assertRaises(AdmissionError): self.check(cap=2)
        self.receipt['files']['prefix.f32']['sha256'] = 'b'*64
        with self.assertRaises(AdmissionError): self.check()
        self.receipt['files']['../escape'] = self.receipt['files'].pop('prefix.f32')
        with self.assertRaises(AdmissionError): self.check()


class NativeNoiseGateTests(unittest.TestCase):
    def comparisons(self, reset=0.0, kl=1e-4, greedy=1.0):
        exact = {name: {'relative_rms': reset} for name in ['prefix.f32', 'norm.f32', 'logits.f32']}
        layout = {name: {'relative_rms': 0.01} for name in ['prefix.f32', 'norm.f32', 'logits.f32']}
        layout['distribution'] = {'max_token_kl': kl, 'mean_token_kl': kl/2, 'greedy_agreement': greedy}
        return {'fresh_reset': exact, 'microbatch': layout}

    def test_measured_layout_noise_with_exact_repeat_passes(self):
        judge_native_comparisons(self.comparisons())

    def test_nonreproducible_clean_repeat_refuses(self):
        with self.assertRaises(ParityError): judge_native_comparisons(self.comparisons(reset=1e-3))

    def test_layout_changing_distributions_refuses(self):
        with self.assertRaises(ParityError): judge_native_comparisons(self.comparisons(kl=2e-3))

    def test_near_tie_argmax_flip_alone_is_recorded_not_fatal(self):
        judge_native_comparisons(self.comparisons(kl=6.5e-4, greedy=34/35))
