"""Read GGUF metadata/tensor directory only; no weights are loaded into RAM or GPU."""
import json
from pathlib import Path
import struct

path = Path(r'D:\ArchieLocal\models\Ternary-Bonsai-2-9B-PTQ1_0.gguf')
formats = {0:'B', 1:'b', 2:'H', 3:'h', 4:'I', 5:'i', 6:'f', 7:'?', 10:'Q', 11:'q', 12:'d'}
with path.open('rb') as stream:
    def unpack(fmt):
        return struct.unpack('<' + fmt, stream.read(struct.calcsize('<' + fmt)))[0]
    def string(keep=True):
        length = unpack('Q')
        if length > 8 * 1024 * 1024:
            raise ValueError('oversized string')
        if keep:
            return stream.read(length).decode('utf-8')
        stream.seek(length, 1)
    def value(kind, keep=True):
        if kind == 8:
            return string(keep)
        if kind == 9:
            item_type, length = unpack('I'), unpack('Q')
            if length > 2_000_000:
                raise ValueError('oversized array')
            if not keep and item_type in formats:
                stream.seek(length * struct.calcsize('<' + formats[item_type]), 1)
                return
            result = [value(item_type, keep) for _ in range(length)]
            return result if keep else None
        return unpack(formats[kind])
    assert stream.read(4) == b'GGUF'
    version, tensor_count, field_count = unpack('I'), unpack('Q'), unpack('Q')
    assert version == 3 and tensor_count < 10_000 and field_count < 10_000
    fields = {}
    for _ in range(field_count):
        key = string()
        kind = unpack('I')
        keep = not key.startswith('tokenizer.')
        result = value(kind, keep)
        if keep:
            fields[key] = result
    tensors = []
    for _ in range(tensor_count):
        name = string()
        dimensions = unpack('I')
        assert dimensions < 5
        shape = [unpack('Q') for _ in range(dimensions)]
        kind, offset = unpack('I'), unpack('Q')
        tensors.append({'name': name, 'shape': shape, 'type': kind, 'offset': offset})
    alignment = fields.get('general.alignment', 32)
    data_offset = (stream.tell() + alignment - 1) // alignment * alignment
report = {'path': str(path), 'bytes': path.stat().st_size, 'fields': fields,
          'tensor_count': tensor_count, 'data_offset': data_offset, 'tensors': tensors}
output = Path(__file__).parent / 'bonsai-9b-gguf-directory.json'
output.write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps({'fields': {key: ({'array_length': len(val)} if isinstance(val, list) else val) for key, val in fields.items()}, 'tensor_count': tensor_count, 'data_offset': data_offset,
                  'sample_tensors': [tensor for tensor in tensors if tensor['name'].startswith('blk.0.') or 'hadamard' in tensor['name'] or 'sign' in tensor['name']]}, indent=2))
