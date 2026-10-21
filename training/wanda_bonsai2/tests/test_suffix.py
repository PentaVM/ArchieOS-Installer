import math
import unittest
import torch
from archie_wanda.suffix import rms_norm, split_query_gate, text_rope, LowRankDelta


class SuffixMathTests(unittest.TestCase):
    def test_rms_norm_uses_mean_squares_and_epsilon(self):
        x=torch.tensor([[3.,4.]],dtype=torch.float64)
        weight=torch.tensor([2.,.5],dtype=torch.float64)
        torch.testing.assert_close(rms_norm(x,weight,1e-6),x/math.sqrt(12.5+1e-6)*weight,
                                   rtol=1e-12,atol=1e-12)

    def test_query_gate_is_interleaved_per_head(self):
        q,g=split_query_gate(torch.arange(16.).reshape(2,8),2,2)
        torch.testing.assert_close(q[0],torch.tensor([[0.,1.],[4.,5.]]))
        torch.testing.assert_close(g[0],torch.tensor([[2.,3.],[6.,7.]]))

    def test_partial_neox_rope_preserves_unrotated_tail(self):
        x=torch.tensor([[[2.,3.,7.,8.]]],dtype=torch.float64)
        y=text_rope(x,torch.tensor([1]),2,10000.)
        expected=torch.tensor([[[2*math.cos(1)-3*math.sin(1),2*math.sin(1)+3*math.cos(1),7,8]]],dtype=torch.float64)
        torch.testing.assert_close(y,expected,rtol=1e-12,atol=1e-12)
        torch.testing.assert_close(text_rope(x,torch.tensor([0]),2,10000.),x)

    def test_delta_zero_baseline_and_both_gradients_after_updates(self):
        torch.manual_seed(17)
        delta=LowRankDelta(3,4,rank=2,alpha=4)
        x=torch.randn(5,3)
        self.assertEqual(delta(x).abs().max().item(),0.)
        optimizer=torch.optim.SGD(delta.parameters(),lr=.1)
        for _ in range(2):
            optimizer.zero_grad(); (delta(x)-1).square().mean().backward(); optimizer.step()
        self.assertGreater(delta.a.grad.abs().max().item(),0.)
        self.assertGreater(delta.b.grad.abs().max().item(),0.)
