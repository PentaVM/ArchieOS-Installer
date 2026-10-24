import hashlib
from pathlib import Path
import struct
import tempfile
import unittest

from archie_wanda.contracts import AdmissionError
from archie_wanda.model import inspect_model, read_tensor_rows
from tests.fixtures import gguf


class IdentityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / 'fixture.gguf'
        gguf(self.path)

    def tearDown(self):
        self.assertEqual(Path(self.temp.name).resolve().parent, Path(tempfile.gettempdir()).resolve())
        self.temp.cleanup()

    def inspect(self):
        return inspect_model(self.path, hashlib.sha256(self.path.read_bytes()).hexdigest())

    def test_valid_read_only_directory_and_rows(self):
        before = self.path.read_bytes()
        model = self.inspect()
        self.assertEqual(model.fields['general.architecture'], 'qwen35')
        self.assertEqual(read_tensor_rows(model, 'fixture.weight', 1, 1).tolist(), [[3., 4.]])
        self.assertEqual(before, self.path.read_bytes())

    def test_wrong_sha_refuses_before_use(self):
        with self.assertRaises(AdmissionError):
            inspect_model(self.path, '0' * 64)

    def test_truncated_tensor_is_rejected(self):
        self.path.write_bytes(self.path.read_bytes()[:-1])
        with self.assertRaises(AdmissionError): self.inspect()

    def test_overlapping_tensors_are_rejected(self):
        gguf(self.path, extra_tensor=True)
        with self.assertRaises(AdmissionError): self.inspect()

    def test_unsupported_type_is_rejected(self):
        gguf(self.path, kind=999)
        with self.assertRaises(AdmissionError): self.inspect()

    def test_impossible_metadata_count_is_bounded(self):
        self.path.write_bytes(b'GGUF' + struct.pack('<IQQ', 3, 1, 2**63))
        with self.assertRaises(AdmissionError): self.inspect()

    def test_changed_base_and_invalid_row_bounds_are_rejected(self):
        model = self.inspect()
        with self.assertRaises(AdmissionError): read_tensor_rows(model, 'fixture.weight', -1, 1)
        with self.assertRaises(AdmissionError): read_tensor_rows(model, 'fixture.weight', 1, 2)
        self.path.write_bytes(self.path.read_bytes() + b'changed')
        with self.assertRaises(AdmissionError): read_tensor_rows(model, 'fixture.weight', 0, 1)
