from pathlib import Path
import tempfile
import unittest
from archie_wanda.contracts import TrainConfig, AdmissionError


class ConfigTests(unittest.TestCase):
    def load(self, extra):
        with tempfile.TemporaryDirectory() as directory:
            assert Path(directory).resolve().parent == Path(tempfile.gettempdir()).resolve()
            path = Path(directory) / 'config.toml'
            path.write_text('model_path="/model.gguf"\n' + extra, encoding='utf-8')
            return TrainConfig.load(path)

    def test_boolean_sequence_length_is_not_a_token_count(self):
        with self.assertRaises(AdmissionError): self.load('sequence_length=true\n')

    def test_unknown_keys_and_lowered_safety_bounds_refuse(self):
        for item in ['unexpected=1', 'minimum_free_disk_bytes=10', 'spare_gpu_mib=0',
                     'cache_cap_bytes=77309411328', 'sequence_length=4097', 'rank=16']:
            with self.subTest(item=item), self.assertRaises(AdmissionError): self.load(item)

    def test_minimal_config_uses_exact_approved_base(self):
        from archie_wanda.contracts import BASE_SHA
        self.assertEqual(self.load('').base_sha256, BASE_SHA)
