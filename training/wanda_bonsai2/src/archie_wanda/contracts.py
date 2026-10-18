from dataclasses import dataclass, field, fields
from pathlib import Path
import hashlib
import json
import math
import tomllib

BASE_SHA = '3561df7ac7d26a71ffa991ac2db740b10841c3a350aa2676ccd3ff5d428b4e8b'
PRISM_COMMIT = 'e3de715222b2667c54a267441159a5729f11155e'


class AdmissionError(ValueError):
    pass


class ParityError(ValueError):
    pass


class ResourceError(RuntimeError):
    pass


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False,
                                    separators=(',', ':'), allow_nan=False).encode()).hexdigest()


@dataclass(frozen=True)
class TensorInfo:
    name: str
    shape: tuple[int, ...]
    kind: int
    offset: int
    nbytes: int


@dataclass(frozen=True)
class ModelIdentity:
    path: Path
    sha256: str
    size: int
    signature: tuple
    data_offset: int
    fields: dict = field(repr=False)
    tensors: dict[str, TensorInfo] = field(repr=False)
    metadata_sha256: str = ''


@dataclass(frozen=True)
class TrainConfig:
    model_path: str
    base_sha256: str = BASE_SHA
    runtime_dir: str = '/opt/archie/llama-cuda'
    run_dir: str = '/mnt/d/ArchieLocal/wanda-training'
    host_system_path: str = '/mnt/c'
    sequence_length: int = 128
    row_chunk: int = 2048
    gpu_ceiling_mib: int = 7168
    spare_gpu_mib: int = 768
    minimum_free_disk_bytes: int = 2147483648
    minimum_available_ram_bytes: int = 1073741824
    cache_cap_bytes: int = 1073741824
    rank: int = 8
    alpha: float = 16.0
    seed: int = 17

    @classmethod
    def load(cls, path):
        with Path(path).open('rb') as stream:
            data = tomllib.load(stream)
        allowed = {f.name for f in fields(cls)}
        if set(data) - allowed:
            raise AdmissionError('unknown training config fields')
        try:
            config = cls(**data)
        except TypeError as error:
            raise AdmissionError('incomplete or invalid training config') from error
        integers = ('sequence_length', 'row_chunk', 'gpu_ceiling_mib', 'spare_gpu_mib',
                    'minimum_free_disk_bytes', 'minimum_available_ram_bytes', 'cache_cap_bytes', 'rank', 'seed')
        if any(type(getattr(config, name)) is not int for name in integers):
            raise AdmissionError('training bounds require integers, not booleans/strings')
        if type(config.alpha) not in (int, float) or not math.isfinite(config.alpha):
            raise AdmissionError('finite LoRA alpha required')
        for name in ('model_path', 'runtime_dir', 'run_dir', 'host_system_path'):
            value = getattr(config, name)
            if not isinstance(value, str) or not value or '\0' in value:
                raise AdmissionError('invalid training path')
        if config.base_sha256 != BASE_SHA or config.rank != 8 or config.alpha != 16:
            raise AdmissionError('initial worker requires exact approved base/rank/alpha')
        # 4096 measured-safe on 8GiB: block-31 attention is ~1GiB at 4096 rows; weights stay packed.
        if not 1 <= config.sequence_length <= 4096 or not 1 <= config.row_chunk <= 4096:
            raise AdmissionError('invalid sequence/chunk bound')
        if not 0 < config.gpu_ceiling_mib <= 7168 or config.spare_gpu_mib < 768:
            raise AdmissionError('GPU safety budget cannot be relaxed')
        if config.minimum_free_disk_bytes < 2**31 or config.minimum_available_ram_bytes < 2**30:
            raise AdmissionError('disk/RAM safety floor cannot be relaxed')
        # Prefix-only training caches are ~16KiB/token; the disk floors above still apply.
        if not 0 < config.cache_cap_bytes <= 64 * 2**30:
            raise AdmissionError('invalid cache cap')
        return config
