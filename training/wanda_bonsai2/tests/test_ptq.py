import hashlib
from pathlib import Path
import tempfile
import unittest
import numpy as np

from archie_wanda.ptq import decode_ptq1
from archie_wanda.contracts import AdmissionError


class DecodeTests(unittest.TestCase):
    def test_extreme_codes_and_scale(self):
        minus = bytes(26) + np.array([2], dtype='<f2').tobytes()
        plus = bytes([255] * 26) + np.array([0.5], dtype='<f2').tobytes()
        np.testing.assert_array_equal(decode_ptq1(minus, stages=(32, 16, 8)), np.full(128, -2.))
        np.testing.assert_array_equal(decode_ptq1(plus, stages=(32, 16, 8)), np.full(128, .5))

    def test_incomplete_block_and_unknown_layout_refuse(self):
        with self.assertRaises(AdmissionError): decode_ptq1(bytes(27))
        with self.assertRaises(AdmissionError): decode_ptq1(bytes(28), stages=(7,))


try:
    import torch
except ImportError:
    torch = None


@unittest.skipIf(torch is None, 'guest Torch environment required')
class LinearTests(unittest.TestCase):
    def test_frozen_chunked_forward_and_input_gradient_match_dense(self):
        from archie_wanda.ptq import FrozenPTQLinear
        from archie_wanda.model import inspect_model
        from tests.fixtures import gguf
        with tempfile.TemporaryDirectory() as directory:
            assert Path(directory).resolve().parent == Path(tempfile.gettempdir()).resolve()
            path = gguf(Path(directory) / 'base.gguf')
            model = inspect_model(path, hashlib.sha256(path.read_bytes()).hexdigest())
            layer = FrozenPTQLinear(model, 'fixture.weight', row_chunk=1)
            x = torch.tensor([[.5, -2.]], requires_grad=True)
            expected_x = x.detach().clone().requires_grad_()
            expected = expected_x @ torch.tensor([[1., 2.], [3., 4.]]).T
            result = layer(x)
            torch.testing.assert_close(result, expected)
            result.square().sum().backward(); expected.square().sum().backward()
            torch.testing.assert_close(x.grad, expected_x.grad)
            self.assertEqual(list(layer.parameters()), [])
