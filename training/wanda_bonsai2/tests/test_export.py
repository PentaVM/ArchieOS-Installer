import hashlib
from pathlib import Path
import tempfile
import unittest
import numpy as np

try:
    import torch
except ImportError:
    torch = None


@unittest.skipIf(torch is None, 'guest Torch environment required')
class ExportTests(unittest.TestCase):
    def test_adapter_round_trips_with_native_layout_and_metadata(self):
        from archie_wanda.export import export_candidate
        from archie_wanda.model import inspect_model, read_tensor_rows
        rng = np.random.default_rng(17)
        a, b = rng.normal(size=(8, 4096)).astype('f4'), rng.normal(size=(8192, 8)).astype('f4')
        with tempfile.TemporaryDirectory() as directory:
            assert Path(directory).resolve().parent == Path(tempfile.gettempdir()).resolve()
            checkpoint, out = Path(directory)/'best.pt', Path(directory)/'candidate.gguf'
            torch.save({'model': {'q_delta.a': torch.from_numpy(a), 'q_delta.b': torch.from_numpy(b)},
                        'settings': {'alpha': 16.0}, 'run_digest': 'r'}, checkpoint)
            receipt = export_candidate(checkpoint, out)
            self.assertEqual(receipt['targets'], ['q'])
            model = inspect_model(out, hashlib.sha256(out.read_bytes()).hexdigest())
            self.assertEqual(model.fields['general.type'], 'adapter')
            self.assertEqual(model.fields['general.architecture'], 'qwen35')
            self.assertEqual(model.fields['adapter.lora.alpha'], 16.0)
            self.assertEqual(model.tensors['blk.31.attn_q.weight.lora_a'].shape, (4096, 8))
            self.assertEqual(model.tensors['blk.31.attn_q.weight.lora_b'].shape, (8, 8192))
            np.testing.assert_array_equal(read_tensor_rows(model, 'blk.31.attn_q.weight.lora_a', 0, 8), a)
            np.testing.assert_array_equal(read_tensor_rows(model, 'blk.31.attn_q.weight.lora_b', 0, 8192), b)
            with self.assertRaises(Exception): export_candidate(checkpoint, out)

    def test_incomplete_or_foreign_tensors_refuse(self):
        from archie_wanda.contracts import AdmissionError
        from archie_wanda.export import export_candidate
        with tempfile.TemporaryDirectory() as directory:
            assert Path(directory).resolve().parent == Path(tempfile.gettempdir()).resolve()
            checkpoint = Path(directory)/'best.pt'
            torch.save({'model': {'q_delta.a': torch.zeros(8, 4096)}}, checkpoint)
            with self.assertRaises(AdmissionError): export_candidate(checkpoint, Path(directory)/'x.gguf')
            torch.save({'model': {'q_delta.a': torch.zeros(8, 4000), 'q_delta.b': torch.zeros(8192, 8)}}, checkpoint)
            with self.assertRaises(AdmissionError): export_candidate(checkpoint, Path(directory)/'y.gguf')
