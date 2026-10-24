import hashlib
from pathlib import Path
import tempfile
import struct
import unittest
import numpy as np
import torch
from archie_wanda.contracts import AdmissionError
from archie_wanda.model import inspect_model
from archie_wanda.ptq import FrozenPTQLinear
from archie_wanda.head_loss import full_vocab_cross_entropy
from tests.fixtures import gguf, string


class HeadLossTests(unittest.TestCase):
    def test_chunked_full_normalization_and_gradient_match_dense_double(self):
        for chunk in (1,3,8):
            with self.subTest(chunk=chunk), tempfile.TemporaryDirectory() as directory:
                assert Path(directory).resolve().parent==Path(tempfile.gettempdir()).resolve()
                weights=np.array([[1.,-2.,3.,4.],[2.,1.,-1.,3.],[4.,2.,1.,0.],[-2.,3.,1.,5.],
                                  [.5,-1.,2.,1.],[3.,2.,1.,4.],[-1.,-2.,3.,4.]],dtype='<f4')
                path=gguf(Path(directory)/'head.gguf',shape=(4,7),raw=weights.tobytes())
                model=inspect_model(path,hashlib.sha256(path.read_bytes()).hexdigest())
                head=FrozenPTQLinear(model,'fixture.weight',row_chunk=chunk)
                x=torch.tensor([[.3,-1.,2.,.2],[.1,.5,-.3,2.],[1.,-1.,-2.,3.]],dtype=torch.float64,requires_grad=True)
                ref_x=x.detach().clone().requires_grad_(); labels=torch.tensor([2,-100,6])
                expected=torch.nn.functional.cross_entropy(ref_x @ torch.from_numpy(weights).double().T,labels)
                actual=full_vocab_cross_entropy(x,labels,head,vocab_chunk=chunk)
                actual.backward(); expected.backward()
                torch.testing.assert_close(actual,expected,rtol=1e-8,atol=1e-8)
                torch.testing.assert_close(x.grad,ref_x.grad,rtol=1e-8,atol=1e-8)
                self.assertEqual(list(head.parameters()),[])

    def test_all_ignored_targets_cannot_count_as_training(self):
        class Head:
            rows=3
        with self.assertRaises(AdmissionError):
            full_vocab_cross_entropy(torch.zeros(2,4),torch.tensor([-100,-100]),Head())

    def test_rotated_head_gradient_uses_transposed_transform(self):
        with tempfile.TemporaryDirectory() as directory:
            assert Path(directory).resolve().parent==Path(tempfile.gettempdir()).resolve()
            rng=np.random.default_rng(17)
            weights=rng.normal(size=(7,4096)).astype('<f4')*.01
            signs=np.where(np.arange(4096)%3,1,-1).astype('i1')
            extra=[('prism.hadamard.weight_names',9,struct.pack('<IQ',8,1)+string('fixture.weight')),
                   ('prism.hadamard.sign_widths',9,struct.pack('<IQI',4,1,4096)),
                   ('prism.hadamard.sign_values',9,struct.pack('<IQ',1,4096)+signs.tobytes())]
            path=gguf(Path(directory)/'rotated.gguf',shape=(4096,7),raw=weights.tobytes(),extra_fields=extra)
            model=inspect_model(path,hashlib.sha256(path.read_bytes()).hexdigest())
            head=FrozenPTQLinear(model,'fixture.weight',row_chunk=3)
            x=torch.from_numpy(rng.normal(size=(2,4096))).requires_grad_()
            reference=x.detach().clone().requires_grad_(); labels=torch.tensor([1,6])
            expected=torch.nn.functional.cross_entropy(head.transform(reference) @ torch.from_numpy(weights).double().T,labels)
            actual=full_vocab_cross_entropy(x,labels,head,vocab_chunk=3)
            actual.backward(); expected.backward()
            torch.testing.assert_close(actual,expected,rtol=1e-8,atol=1e-8)
            torch.testing.assert_close(x.grad,reference.grad,rtol=1e-8,atol=1e-8)
