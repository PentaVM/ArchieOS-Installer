"""Writes `tiny-release.pt`: a release-shaped PyTorch zip without PyTorch.

The layout is what `torch.save` writes (protocol 2 pickle in `archive/data.pkl`, one raw
storage per key under `archive/data/`), with stand-in `torch` modules so the pickle names
`torch._utils._rebuild_tensor_v2` and `torch.FloatStorage` exactly as a real release does.
Two tensors share one storage at different offsets, one storage is bfloat16, and
`read_decoder.*` stands for a head the runtime never asks for.

    python3 -I make_tiny_release.py <out.pt>
"""

import collections
import pickle
import struct
import sys
import types
import zipfile

torch = types.ModuleType("torch")
utils = types.ModuleType("torch._utils")


def _rebuild_tensor_v2(*args):
    raise NotImplementedError


_rebuild_tensor_v2.__module__ = "torch._utils"
utils._rebuild_tensor_v2 = _rebuild_tensor_v2


class FloatStorage:
    pass


class BFloat16Storage:
    pass


for cls in (FloatStorage, BFloat16Storage):
    cls.__module__ = "torch"
    setattr(torch, cls.__name__, cls)
torch._utils = utils
sys.modules["torch"] = torch
sys.modules["torch._utils"] = utils


class Storage:
    def __init__(self, key, cls, values):
        self.key, self.cls, self.values = key, cls, values

    def raw(self):
        if self.cls is FloatStorage:
            return struct.pack(f"<{len(self.values)}f", *self.values)
        out = bytearray()
        for value in self.values:
            bits = struct.unpack("<I", struct.pack("<f", value))[0]
            bits += 0x7FFF + ((bits >> 16) & 1)  # round to nearest even
            out += struct.pack("<H", (bits >> 16) & 0xFFFF)
        return bytes(out)


class Tensor:
    def __init__(self, storage, offset, size):
        self.storage, self.offset, self.size = storage, offset, size

    def __reduce__(self):
        stride, step = [], 1
        for dim in reversed(self.size):
            stride.insert(0, step)
            step *= dim
        return (
            _rebuild_tensor_v2,
            (self.storage, self.offset, tuple(self.size), tuple(stride), False,
             collections.OrderedDict()),
        )


class Pickler(pickle.Pickler):
    def persistent_id(self, obj):
        if isinstance(obj, Storage):
            return ("storage", obj.cls, obj.key, "cpu", len(obj.values))
        return None


shared = Storage("0", FloatStorage, [0.5 * i - 3.0 for i in range(40)])
halves = Storage("1", BFloat16Storage, [0.25 * i - 1.0 for i in range(12)])
unused = Storage("2", FloatStorage, [1.0] * 6)
state = {
    "fused_norm.weight": Tensor(shared, 0, [8]),
    "fused_norm.bias": Tensor(shared, 8, [8]),
    "fusion.0.moe.b_in": Tensor(shared, 16, [2, 1, 12]),
    "position": Tensor(halves, 0, [3, 4]),
    "read_decoder.weight": Tensor(unused, 0, [2, 3]),
}
top = {
    "format": "archie-iris-beta1-weights-v1",
    "step": 7,
    "score": 0.5,
    "model_config": {"backbone": "dual", "image_size": 64, "dimension": 8, "heads": 2,
                     "experts": 2, "expert_top_k": 1},
    "model": state,
}
with zipfile.ZipFile(sys.argv[1], "w", zipfile.ZIP_STORED) as archive:
    with archive.open("archive/data.pkl", "w") as handle:
        Pickler(handle, protocol=2).dump(top)
    for storage in (shared, halves, unused):
        archive.writestr(f"archive/data/{storage.key}", storage.raw())
    archive.writestr("archive/version", "3\n")
