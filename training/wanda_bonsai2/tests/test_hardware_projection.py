"""Opt-in real checkpoint/CUDA proof; never included in routine CPU discovery."""
import ctypes
from dataclasses import asdict
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import unittest
import numpy as np


@unittest.skipUnless(os.environ.get('ARCHIE_WANDA_HARDWARE') == '1', 'explicit real-hardware opt-in required')
class ProjectionHardwareTests(unittest.TestCase):
    def test_real_prism_unpack_frozen_forward_and_input_gradient(self):
        import torch
        from archie_wanda.contracts import TrainConfig
        from archie_wanda.model import inspect_model, read_tensor_rows, signature
        from archie_wanda.ptq import FrozenPTQLinear
        from archie_wanda.resources import preflight
        config = TrainConfig.load('configs/rtx3070-smoke.toml')
        resources = preflight(config, required_gpu_mib=512)
        model = inspect_model(Path(config.model_path), config.base_sha256)
        name = 'blk.31.attn_v.weight'
        tensor = model.tensors[name]
        decoded = read_tensor_rows(model, name, 0, 1024)
        with model.path.open('rb') as stream:
            stream.seek(model.data_offset + tensor.offset)
            raw = stream.read(tensor.nbytes)
        library = ctypes.CDLL(config.runtime_dir + '/libggml-base.so')  # CPU-only unpacker
        unpack = library.dequantize_row_ptq1_0
        unpack.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int64]
        reference = np.empty_like(decoded)
        unpack(ctypes.c_char_p(raw), ctypes.c_void_p(reference.ctypes.data), reference.size)
        np.testing.assert_array_equal(decoded, reference)
        torch.manual_seed(17)
        torch.cuda.reset_peak_memory_stats()
        layer = FrozenPTQLinear(model, name, row_chunk=512)
        x = torch.randn(8, 4096, device='cuda', requires_grad=True)
        baseline_x = x.detach().clone().requires_grad_()
        weight = torch.from_numpy(decoded).to('cuda')
        expected = layer.transform(baseline_x) @ weight.T
        actual = layer(x)
        torch.testing.assert_close(actual, expected, rtol=2e-5, atol=2e-5)
        actual.square().mean().backward(); expected.square().mean().backward()
        torch.testing.assert_close(x.grad, baseline_x.grad, rtol=2e-5, atol=2e-6)
        self.assertEqual(list(layer.parameters()), [])
        self.assertEqual(signature(model.path), model.signature)
        self.assertTrue(torch.isfinite(x.grad).all().item())
        output = Path('../../_dev/verification/rtx3070/results')
        output.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
        receipt = {'format_version': 1, 'scope': 'maintained_one_projection_forward_input_gradient',
                   'base_sha256': model.sha256, 'tensor': name, 'prism_unpack_bit_identical': True,
                   'forward_max_abs': (actual-expected).abs().max().item(),
                   'gradient_max_abs': (x.grad-baseline_x.grad).abs().max().item(),
                   'peak_allocated_mib': torch.cuda.max_memory_allocated()/2**20,
                   'device': torch.cuda.get_device_name(), 'resources': asdict(resources),
                   'full_model_backward': False, 'wanda_training': False}
        with (output / f'{stamp}-wanda-maintained-projection.json').open('x') as stream:
            json.dump(receipt, stream, indent=2)
        print(json.dumps(receipt))
