"""Export a trained block-31 LoRA as a native Prism/llama.cpp GGUF adapter (unpromoted).

Prism's build_lora_mm adds B·A·x on the *original* activation, before its Hadamard
fold, with scale alpha/rank: the same basis and scale as suffix.LowRankDelta, so the
tensors are written unchanged. A is stored ne=[in, rank], B ne=[rank, out].
"""
import hashlib
import json
from pathlib import Path
import struct
import numpy as np
from .contracts import BASE_SHA, AdmissionError
from .suffix import TARGETS

ALIGNMENT = 32


def _string(value):
    raw = value.encode('utf-8')
    return struct.pack('<Q', len(raw)) + raw


def _kv(key, value):
    if isinstance(value, str):
        return _string(key) + struct.pack('<I', 8) + _string(value)
    if isinstance(value, float):
        return _string(key) + struct.pack('<If', 6, value)
    raise AdmissionError('unsupported adapter metadata type')


def write_adapter(tensors, metadata, destination):
    """tensors: ordered list of (name, float32 array shaped [ne1, ne0])."""
    destination = Path(destination)
    if destination.exists():
        raise AdmissionError('refusing to overwrite an existing candidate')
    header = b'GGUF' + struct.pack('<IQQ', 3, len(tensors), len(metadata))
    header += b''.join(_kv(k, v) for k, v in metadata.items())
    offset, infos, blobs = 0, b'', []
    for name, array in tensors:
        array = np.ascontiguousarray(array, dtype='<f4')
        if array.ndim != 2 or not np.isfinite(array).all():
            raise AdmissionError('adapter tensor must be a finite matrix')
        ne0, ne1 = array.shape[1], array.shape[0]
        infos += _string(name) + struct.pack('<IQQIQ', 2, ne0, ne1, 0, offset)
        blob = array.tobytes()
        blobs.append(blob + b'\0' * (-len(blob) % ALIGNMENT))
        offset += len(blobs[-1])
    head = header + infos
    head += b'\0' * (-len(head) % ALIGNMENT)
    temporary = destination.with_name(destination.name + '.partial')
    temporary.write_bytes(head + b''.join(blobs))
    temporary.replace(destination)
    return hashlib.sha256(destination.read_bytes()).hexdigest()


def export_candidate(checkpoint, destination):
    import torch
    saved = torch.load(checkpoint, weights_only=True, map_location='cpu')
    state = saved['model']
    targets = sorted({key.split('_delta.')[0] for key in state}, key='qkvo'.index)
    if not targets or set(targets) - set(TARGETS) or len(state) != 2 * len(targets):
        raise AdmissionError('checkpoint does not hold complete supported LoRA pairs')
    tensors = []
    for target in targets:
        name, columns, rows = TARGETS[target]
        a, b = state[target + '_delta.a'].float().numpy(), state[target + '_delta.b'].float().numpy()
        if a.shape[1] != columns or b.shape[0] != rows or a.shape[0] != b.shape[1]:
            raise AdmissionError('LoRA shapes do not match the exact block-31 geometry')
        tensors += [(f'blk.31.{name}.weight.lora_a', a), (f'blk.31.{name}.weight.lora_b', b)]
    settings = saved.get('settings', {})
    metadata = {'general.architecture': 'qwen35', 'general.type': 'adapter', 'adapter.type': 'lora',
                'adapter.lora.alpha': float(settings.get('alpha', 16.0)),
                'general.name': 'wanda-candidate-unpromoted',
                'archie.wanda.base_sha256': BASE_SHA,
                'archie.wanda.run_digest': str(saved.get('run_digest', '')),
                'archie.wanda.targets': ','.join(targets),
                'archie.wanda.method': 'native-quantized-base LoRA, block 31; not canonical QLoRA'}
    digest = write_adapter(tensors, metadata, destination)
    receipt = {'format_version': 1, 'candidate': str(destination), 'sha256': digest,
               'checkpoint': str(checkpoint), 'targets': targets, 'base_sha256': BASE_SHA,
               'run_digest': saved.get('run_digest'), 'metrics': saved.get('metrics'), 'promoted': False}
    Path(str(destination) + '.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt
