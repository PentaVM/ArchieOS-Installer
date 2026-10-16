"""Prism PTQ decoding; Torch is imported only when training operators are used."""
import numpy as np
from .contracts import AdmissionError


def decode_ptq1(raw: bytes, stages=(32, 16, 8)) -> np.ndarray:
    if not raw or len(raw) % 28 or tuple(stages) != (32, 16, 8):
        raise AdmissionError('unsupported PTQ1_0 block/layout')
    blocks = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 28)
    parts, index = [], 0
    for width in stages:
        while index + width <= 24:
            for power in range(5):
                wrapped = (blocks[:, index:index+width].astype(np.uint16) * 3**power) & 255
                parts.append(((wrapped * 3) >> 8).astype(np.int16) - 1)
            index += width
    for power in range(4):
        wrapped = (blocks[:, 24:26].astype(np.uint16) * 3**power) & 255
        parts.append(((wrapped * 3) >> 8).astype(np.int16) - 1)
    scales = blocks[:, 26:28].copy().view('<f2').astype(np.float32)
    return (np.concatenate(parts, axis=1).astype(np.float32) * scales).reshape(-1)


def decode_ptq1_torch(blocks):
    """Device-side twin of decode_ptq1 for packed [N,28] uint8 blocks; same integer math."""
    import torch
    if blocks.dtype != torch.uint8 or blocks.ndim != 2 or blocks.shape[1] != 28:
        raise AdmissionError('unsupported PTQ1_0 block/layout')
    codes = blocks[:, :26].to(torch.int32)
    parts = []
    for first, last, powers in ((0, 16, 5), (16, 24, 5), (24, 26, 4)):
        segment = codes[:, first:last]
        for power in range(powers):
            parts.append((((segment * 3**power) & 255) * 3 >> 8) - 1)
    scales = blocks[:, 26:28].contiguous().view(torch.float16).to(torch.float32)
    return torch.cat(parts, dim=1).to(torch.float32) * scales


def __getattr__(name):
    if name != 'FrozenPTQLinear':
        raise AttributeError(name)
    # Parser/dequantization and native extraction must not load the Torch CUDA stack.
    import torch
    from .model import read_tensor_rows, read_tensor_bytes

    class Matmul(torch.autograd.Function):
        @staticmethod
        def forward(ctx, x, owner):
            ctx.owner, ctx.shape = owner, x.shape
            flat = owner.transform(x.reshape(-1, owner.columns))
            out = torch.empty((flat.shape[0], owner.rows), device=x.device, dtype=x.dtype)
            for first in range(0, owner.rows, owner.row_chunk):
                weight = owner.chunk(first, x)
                out[:, first:first+weight.shape[0]] = flat @ weight.T
            return out.reshape(*x.shape[:-1], owner.rows)

        @staticmethod
        def backward(ctx, gradient):
            owner = ctx.owner
            flat = gradient.reshape(-1, owner.rows)
            dx = torch.zeros((flat.shape[0], owner.columns), device=flat.device, dtype=flat.dtype)
            for first in range(0, owner.rows, owner.row_chunk):
                weight = owner.chunk(first, flat)
                dx.addmm_(flat[:, first:first+weight.shape[0]], weight)
            return owner.transform_transpose(dx).reshape(ctx.shape), None

    class FrozenPTQLinear(torch.nn.Module):
        def __init__(self, model, tensor_name, row_chunk=2048):
            super().__init__()
            tensor = model.tensors[tensor_name]
            if len(tensor.shape) != 2 or not 1 <= row_chunk <= 4096:
                raise AdmissionError('linear requires a matrix and bounded chunks')
            self.model, self.name, self.row_chunk = model, tensor_name, row_chunk
            self.columns, self.rows = tensor.shape
            self.kind, self.packed = tensor.kind, None
            self.rotated = tensor_name in model.fields.get('prism.hadamard.weight_names', [])
            self.sign_values = None
            if self.rotated:
                widths = model.fields.get('prism.hadamard.sign_widths', [])
                if self.columns not in widths or self.columns % 4096:
                    raise AdmissionError('unsupported Hadamard input width')
                start = sum(widths[:widths.index(self.columns)])
                self.sign_values = model.fields['prism.hadamard.sign_values'][start:start+self.columns]
                if len(self.sign_values) != self.columns or not all(v in (-1, 1) for v in self.sign_values):
                    raise AdmissionError('invalid explicit Hadamard signs')

        def make_resident(self, device):
            """Keep the frozen packed bytes (not a dense copy) on the device for repeated passes."""
            if self.kind != 143:
                return self
            raw = read_tensor_bytes(self.model, self.name)
            self.packed = torch.frombuffer(bytearray(raw), dtype=torch.uint8).to(device).reshape(self.rows, -1)
            return self

        def chunk(self, first, like, count=None):
            count = min(count or self.row_chunk, self.rows-first)
            if self.packed is not None and self.packed.device == like.device:
                blocks = self.packed[first:first+count].reshape(-1, 28)
                return decode_ptq1_torch(blocks).reshape(count, self.columns).to(like.dtype)
            values = read_tensor_rows(self.model, self.name, first, count)
            return torch.from_numpy(values).to(device=like.device, dtype=like.dtype)

        @staticmethod
        def hadamard(value):
            shape = value.shape
            value = value.reshape(-1, 4096)
            width = 1
            while width < 4096:
                pairs = value.reshape(value.shape[0], -1, width*2)
                left, right = pairs[..., :width], pairs[..., width:]
                value = torch.cat((left+right, left-right), dim=-1).reshape(-1, 4096)
                width *= 2
            return (value / 64).reshape(shape)

        def transform(self, value):
            if not self.rotated: return value
            signs = torch.tensor(self.sign_values, device=value.device, dtype=value.dtype)
            return self.hadamard(value * signs)

        def transform_transpose(self, value):
            if not self.rotated: return value
            signs = torch.tensor(self.sign_values, device=value.device, dtype=value.dtype)
            return self.hadamard(value) * signs

        def forward(self, x):
            if x.shape[-1] != self.columns or not x.is_floating_point():
                raise AdmissionError('linear input layout mismatch')
            return Matmul.apply(x, self)

    globals()['FrozenPTQLinear'] = FrozenPTQLinear
    return FrozenPTQLinear
