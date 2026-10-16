"""Small owned GGUF fixtures: file-contract tests, not model-quality evidence."""
import struct


def string(value):
    raw = value.encode()
    return struct.pack('<Q', len(raw)) + raw


def gguf(path, *, kind=0, shape=(2, 2), raw=None, offset=0, extra_tensor=False):
    fields = [('general.architecture', 8, string('qwen35')),
              ('general.alignment', 4, struct.pack('<I', 32))]
    data = b'GGUF' + struct.pack('<IQQ', 3, 2 if extra_tensor else 1, len(fields))
    for name, tag, value in fields:
        data += string(name) + struct.pack('<I', tag) + value
    for name in (['fixture.weight', 'overlap.weight'] if extra_tensor else ['fixture.weight']):
        data += string(name) + struct.pack('<I', len(shape))
        data += struct.pack('<' + 'Q' * len(shape), *shape) + struct.pack('<IQ', kind, offset)
    data += bytes((-len(data)) % 32)
    if raw is None:
        raw = struct.pack('<4f', 1, 2, 3, 4)
    path.write_bytes(data + raw)
    return path
