"""Throwaway compatibility probe, NOT a Wanda trainer or a full 9B fine-tune.

Uses one real PTQ1_0 tensor, checks unpacking against the installed Prism library,
checks Hadamard algebra, and optimizes tiny LoRA matrices on random activations.
Artifacts are unpromoted probes and must never be offered as a learned Wanda model.
"""
import ctypes
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import numpy as np
import torch
import torch.nn.functional as F

torch.set_num_threads(4)
torch.manual_seed(42)
root = Path('/mnt/c/Users/Penta/Archie-RTX3070-Handoff/local-verification')
directory = json.loads((root / 'bonsai-9b-gguf-directory.json').read_text())
path = Path('/mnt/d/ArchieLocal/models/Ternary-Bonsai-2-9B-PTQ1_0.gguf')
out = Path('/mnt/d/ArchieLocal/compatibility-probes')
out.mkdir(exist_ok=True)
report_path = Path('/mnt/c/Users/Penta/Documents/ChatGPT/ArchieOS/_dev/verification/rtx3070/results/2026-10-16-bonsai-lora-layer-probe.json')
assert not report_path.exists()
info = next(t for t in directory['tensors'] if t['name'] == 'blk.31.attn_v.weight')
assert info['type'] == 143 and info['shape'] == [4096, 1024]
before = path.stat()
with path.open('rb') as stream:
    digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    stream.seek(directory['data_offset'] + info['offset'])
    raw = stream.read(math.prod(info['shape']) // 128 * 28)
blocks = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 28)
pieces = []
j = 0
for c in (32, 16, 8):
    while j + c <= 24:
        for n in range(5):
            q = (blocks[:, j:j+c].astype(np.uint16) * (3 ** n)) & 255
            pieces.append(((q * 3) >> 8).astype(np.int16) - 1)
        j += c
for n in range(4):
    q = (blocks[:, 24:26].astype(np.uint16) * (3 ** n)) & 255
    pieces.append(((q * 3) >> 8).astype(np.int16) - 1)
scales = blocks[:, 26:28].copy().view('<f2').astype(np.float32)
decoded = (np.concatenate(pieces, axis=1).astype(np.float32) * scales).reshape(1024, 4096)
reference = np.empty_like(decoded)
library = ctypes.CDLL('/opt/archie/llama-cuda/libggml-base.so')
dequantize = library.dequantize_row_ptq1_0
dequantize.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int64]
dequantize.restype = None
dequantize(ctypes.c_char_p(raw), ctypes.c_void_p(reference.ctypes.data), reference.size)
assert np.array_equal(decoded, reference), 'PTQ unpack differs from actual Prism runtime'

device = torch.device('cuda')
torch.cuda.reset_peak_memory_stats()
weight = torch.from_numpy(decoded.copy()).to(device)
original_weight_hash = hashlib.sha256(decoded.tobytes()).hexdigest()
fields = directory['fields']
assert fields['prism.hadamard.sign_widths'][0] == 4096
signs = torch.tensor(fields['prism.hadamard.sign_values'][:4096], dtype=torch.float32, device=device)
assert info['name'] in fields['prism.hadamard.weight_names']
def hadamard(tensor):
    shape = tensor.shape
    value = tensor.reshape(-1, 4096)
    width = 1
    while width < 4096:
        pairs = value.reshape(value.shape[0], -1, width * 2)
        left, right = pairs[..., :width], pairs[..., width:]
        value = torch.cat((left + right, left - right), dim=-1).reshape(-1, 4096)
        width *= 2
    return (value / 64).reshape(shape)

x = torch.randn(16, 4096, device=device)
with torch.no_grad():
    base = F.linear(hadamard(x * signs), weight)
    effective = hadamard(weight) * signs
    reconstructed = F.linear(x, effective)
    algebra_error = ((base - reconstructed).abs().max() / base.abs().max()).item()
assert algebra_error < 2e-5, algebra_error
rank, alpha = 8, 16.0
a = torch.nn.Parameter(torch.randn(rank, 4096, device=device) * 0.005)
b = torch.nn.Parameter(torch.zeros(1024, rank, device=device))
with torch.no_grad():
    teacher_a = torch.randn(rank, 4096, device=device) * 0.005
    teacher_b = torch.randn(1024, rank, device=device) * 0.01
    target = base + F.linear(F.linear(x, teacher_a), teacher_b) * (alpha / rank)
optimizer = torch.optim.AdamW([a, b], lr=0.001, weight_decay=0)
losses = []
for _ in range(50):
    optimizer.zero_grad(set_to_none=True)
    prediction = base + F.linear(F.linear(x, a), b) * (alpha / rank)
    loss = F.mse_loss(prediction, target)
    loss.backward()
    assert torch.isfinite(a.grad).all() and torch.isfinite(b.grad).all()
    optimizer.step()
    losses.append(loss.item())
assert losses[-1] < losses[0] * 0.5, losses
assert hashlib.sha256(weight.cpu().numpy().tobytes()).hexdigest() == original_weight_hash

def gguf_string(text):
    encoded = text.encode('utf-8')
    return struct.pack('<Q', len(encoded)) + encoded
def save_adapter(filename, b_value):
    destination = out / filename
    assert not destination.exists()
    a_array, b_array = a.detach().cpu().numpy(), b_value.detach().cpu().numpy()
    metadata = [('general.type', 8, 'adapter'), ('general.architecture', 8, 'qwen35'),
                ('general.name', 8, 'COMPATIBILITY-PROBE-NOT-WANDA-TRAINING'),
                ('adapter.type', 8, 'lora'), ('adapter.lora.alpha', 6, alpha)]
    payload = bytearray(b'GGUF' + struct.pack('<IQQ', 3, 2, len(metadata)))
    for name, kind, value in metadata:
        payload += gguf_string(name) + struct.pack('<I', kind)
        payload += gguf_string(value) if kind == 8 else struct.pack('<f', value)
    offset = 0
    for suffix, array in (('lora_a', a_array), ('lora_b', b_array)):
        payload += gguf_string(info['name'] + '.' + suffix)
        payload += struct.pack('<IQQIQ', 2, array.shape[1], array.shape[0], 0, offset)
        offset += array.nbytes
    payload += b'\0' * ((-len(payload)) % 32)
    with destination.open('xb') as stream:
        stream.write(payload)
        stream.write(a_array.tobytes())
        stream.write(b_array.tobytes())
    return str(destination)
zero_path = save_adapter('bonsai9b-zero-lora-PROBE.gguf', torch.zeros_like(b))
trained_path = save_adapter('bonsai9b-one-layer-lora-PROBE.gguf', b)
after = path.stat()
assert (before.st_size, before.st_mtime_ns) == (after.st_size, after.st_mtime_ns)
report = {'scope': 'one_real_projection_only', 'full_model_training': False, 'wanda_training': False,
          'synthetic_activations_and_targets': True, 'source_base_sha256': digest,
          'source_model': fields['general.name'], 'tensor': info, 'ptq_decode_matches_runtime': True,
          'hadamard_algebra_relative_max': algebra_error, 'cuda_device': torch.cuda.get_device_name(),
          'rank': rank, 'alpha': alpha, 'first_loss': losses[0], 'last_loss': losses[-1],
          'base_tensor_unchanged': True, 'peak_allocated_mib': torch.cuda.max_memory_allocated()/1024**2,
          'zero_adapter': zero_path, 'nonzero_adapter': trained_path, 'promoted': False}
report_path.write_text(json.dumps(report, indent=2))
print(json.dumps(report, indent=2))
