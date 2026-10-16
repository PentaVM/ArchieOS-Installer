"""Bounded, read-only GGUF3 parser for the pinned F32/PTQ1_0 checkpoint."""
import hashlib
import math
from pathlib import Path
import struct
import numpy as np

from .contracts import AdmissionError, ModelIdentity, TensorInfo, digest

FORMATS = {0: '<B', 1: '<b', 2: '<H', 3: '<h', 4: '<I', 5: '<i',
           6: '<f', 10: '<Q', 11: '<q', 12: '<d'}


def signature(path):
    value = Path(path).stat()
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns)


class Reader:
    def __init__(self, stream, size):
        self.stream, self.size = stream, size

    def take(self, count):
        if count < 0 or count > self.size - self.stream.tell():
            raise AdmissionError('truncated GGUF')
        value = self.stream.read(count)
        if len(value) != count:
            raise AdmissionError('truncated GGUF')
        return value

    def number(self, fmt):
        return struct.unpack(fmt, self.take(struct.calcsize(fmt)))[0]

    def string(self):
        count = self.number('<Q')
        if count > 32 * 1024 * 1024:
            raise AdmissionError('GGUF string exceeds limit')
        try:
            return self.take(count).decode('utf-8')
        except UnicodeError as error:
            raise AdmissionError('invalid GGUF string') from error

    def value(self, tag):
        if tag in FORMATS:
            return self.number(FORMATS[tag])
        if tag == 7:
            value = self.number('<B')
            if value not in (0, 1): raise AdmissionError('invalid boolean')
            return bool(value)
        if tag == 8:
            return self.string()
        if tag == 9:
            item, count = self.number('<I'), self.number('<Q')
            if item == 9 or count > 1048576:
                raise AdmissionError('unsupported/oversized GGUF array')
            return [self.value(item) for _ in range(count)]
        raise AdmissionError('unsupported GGUF metadata type')


def inspect_model(path: Path, expected_sha: str) -> ModelIdentity:
    path = Path(path).resolve()
    before = signature(path)
    if len(expected_sha) != 64:
        raise AdmissionError('expected SHA256 required')
    with path.open('rb') as stream:
        actual = hashlib.file_digest(stream, 'sha256').hexdigest()
        if actual != expected_sha:
            raise AdmissionError('base SHA256 mismatch')
        stream.seek(0)
        reader = Reader(stream, before[2])
        if reader.take(4) != b'GGUF' or reader.number('<I') != 3:
            raise AdmissionError('only GGUF version3 is supported')
        tensor_count, field_count = reader.number('<Q'), reader.number('<Q')
        if not 0 < tensor_count <= 16384 or field_count > 8192:
            raise AdmissionError('oversized GGUF directory')
        metadata = {}
        for _ in range(field_count):
            name = reader.string()
            if not name or name in metadata:
                raise AdmissionError('duplicate/empty metadata name')
            metadata[name] = reader.value(reader.number('<I'))
        alignment = metadata.get('general.alignment', 32)
        if not isinstance(alignment, int) or alignment <= 0 or alignment > 1048576 or alignment & (alignment - 1):
            raise AdmissionError('invalid GGUF alignment')
        tensors = {}
        for _ in range(tensor_count):
            name, ndim = reader.string(), reader.number('<I')
            if not name or name in tensors or not 1 <= ndim <= 4:
                raise AdmissionError('invalid tensor directory')
            shape = tuple(reader.number('<Q') for _ in range(ndim))
            kind, offset = reader.number('<I'), reader.number('<Q')
            elements = math.prod(shape)
            if not all(shape) or elements > 2**34 or offset % alignment:
                raise AdmissionError('invalid tensor range/alignment')
            if kind == 0:
                nbytes = elements * 4
            elif kind == 143 and shape[0] % 128 == 0:
                nbytes = elements // 128 * 28
            else:
                raise AdmissionError(f'unsupported tensor type/layout: {kind}')
            tensors[name] = TensorInfo(name, shape, kind, offset, nbytes)
        data_offset = (stream.tell() + alignment - 1) // alignment * alignment
        previous_end = 0
        for tensor in sorted(tensors.values(), key=lambda value: value.offset):
            if tensor.offset < previous_end or data_offset + tensor.offset + tensor.nbytes > before[2]:
                raise AdmissionError('overlapping/truncated tensor range')
            previous_end = tensor.offset + tensor.nbytes
    if signature(path) != before:
        raise AdmissionError('base changed during identity check')
    try:
        metadata_digest = digest(metadata)
    except ValueError as error:
        raise AdmissionError('nonfinite metadata') from error
    return ModelIdentity(path, actual, before[2], before, data_offset,
                         metadata, tensors, metadata_digest)


def read_tensor_rows(model: ModelIdentity, name: str, first: int, count: int) -> np.ndarray:
    if signature(model.path) != model.signature:
        raise AdmissionError('base changed after verification')
    if name not in model.tensors:
        raise AdmissionError('unknown tensor')
    tensor = model.tensors[name]
    columns, rows = tensor.shape[0], math.prod(tensor.shape[1:])
    if first < 0 or count <= 0 or first + count > rows or count * columns * 4 > 256 * 1024 * 1024:
        raise AdmissionError('tensor chunk outside bounded row range')
    row_bytes = columns * 4 if tensor.kind == 0 else columns // 128 * 28
    with model.path.open('rb') as stream:
        stream.seek(model.data_offset + tensor.offset + first * row_bytes)
        raw = stream.read(count * row_bytes)
    if len(raw) != count * row_bytes or signature(model.path) != model.signature:
        raise AdmissionError('base changed while reading tensor')
    if tensor.kind == 0:
        values = np.frombuffer(raw, dtype='<f4').copy()
    else:
        from .ptq import decode_ptq1
        values = decode_ptq1(raw, tuple(model.fields.get('prism.ptq1_0.stages', [32, 16, 8])))
    if not np.isfinite(values).all():
        raise AdmissionError('nonfinite base weight')
    return values.reshape(count, columns)
