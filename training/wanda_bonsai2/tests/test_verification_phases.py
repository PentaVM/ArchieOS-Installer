from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch
from archie_wanda.contracts import TrainConfig
from archie_wanda.verification import verify_native


class VerificationPhases(unittest.TestCase):
    def test_all_native_children_exit_before_torch_suffix_can_allocate(self):
        class StopAtTorch(Exception):
            pass
        events = []
        def native(*args, **kwargs):
            events.append('native_lora' if len(args) == 6 else 'native_base')
            return {'tokens': list(range(10))}
        def suffix(*args, **kwargs):
            events.append('torch_suffix')
            raise StopAtTorch()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cases = root / 'cases.jsonl'
            cases.write_text('{"id":"public","text":"public case"}\n')
            config = TrainConfig(model_path='/unused', run_dir=str(root))
            with patch('archie_wanda.verification.inspect_model', return_value=SimpleNamespace(path='/unused')), \
                 patch('archie_wanda.native.build_native', return_value='binary'), \
                 patch('archie_wanda.verification._native_run', side_effect=native), \
                 patch('archie_wanda.suffix.BonsaiSuffix', side_effect=suffix), \
                 patch('torch.load', return_value={'model': {'q_delta.a': 0, 'q_delta.b': 0}}):
                with self.assertRaises(StopAtTorch):
                    verify_native(config, 'headers', 'candidate', 'checkpoint', cases, root / 'out.json')
        self.assertEqual(events, ['native_base', 'native_lora', 'torch_suffix'])
