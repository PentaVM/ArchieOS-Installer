import os
import unittest
from unittest.mock import patch
from archie_wanda import native
from archie_wanda.contracts import AdmissionError, TrainConfig, ParityError
from archie_wanda.verification import suffix_criterion


class StrictReferenceTests(unittest.TestCase):
    def test_profile_disables_activation_quantization_and_tf32_only_in_child(self):
        self.assertTrue(hasattr(native, 'STRICT_F32_ENV'), 'strict native profile is missing')
        with patch.dict(os.environ, {'GGML_CUDA_CUBLAS_COMPUTE_TYPE': 'f16'}):
            environment = native.native_environment(TrainConfig(model_path='/unused'), 'strict-f32')
            self.assertEqual(environment['GGML_CUDA_CUBLAS_COMPUTE_TYPE'], 'f32')
            self.assertEqual(environment['GGML_CUDA_PTQ1_0_MMQ_MAX_BATCH'], '0')
            self.assertEqual(environment['NVIDIA_TF32_OVERRIDE'], '0')
            self.assertEqual(os.environ['GGML_CUDA_CUBLAS_COMPUTE_TYPE'], 'f16')

    def test_balanced_microbatches_never_enter_four_token_q8_path(self):
        self.assertTrue(hasattr(native, 'strict_microbatch'), 'balanced strict layout is missing')
        for count in (10, 19, 35):
            width = native.strict_microbatch(count)
            self.assertGreaterEqual(count - width, 5)
            self.assertGreaterEqual(width, 5)
            self.assertLess(width, count)
        with self.assertRaises(AdmissionError):
            native.strict_microbatch(9)

    def test_strict_profile_cannot_pass_by_native_layout_noise(self):
        floor = {'norm_relative_rms': .02, 'max_token_kl': .01}
        self.assertIsNone(suffix_criterion(.0032, .0001, floor, 'strict-f32'))
        self.assertEqual(suffix_criterion(.0004, .0001, floor, 'strict-f32'), 'strict')

    def test_strict_microbatch_capture_requires_original_tolerance(self):
        exact = {name: {'relative_rms': 0.} for name in ('prefix.f32', 'norm.f32', 'logits.f32')}
        layout = {name: {'relative_rms': .0032} for name in exact}
        layout['distribution'] = {'max_token_kl': .0001}
        with self.assertRaises(ParityError):
            native.judge_native_comparisons({'fresh_reset': exact, 'microbatch': layout}, 'strict-f32')

    def test_nan_negative_or_boolean_native_metrics_cannot_pass(self):
        for bad in (float('nan'), float('inf'), -1., True):
            exact = {name: {'relative_rms': 0.} for name in ('prefix.f32','norm.f32','logits.f32')}
            layout = {name: {'relative_rms': bad} for name in exact}
            layout['distribution'] = {'max_token_kl': 0.}
            with self.subTest(value=bad), self.assertRaises(ParityError):
                native.judge_native_comparisons({'fresh_reset': exact, 'microbatch': layout}, 'strict-f32')
