from dataclasses import dataclass, asdict
from pathlib import Path
import shutil
import subprocess
from .contracts import ResourceError


@dataclass(frozen=True)
class ResourceSnapshot:
    total_gpu_mib: int
    used_gpu_mib: int
    free_gpu_mib: int
    worker_budget_mib: int
    available_ram_bytes: int
    guest_free_disk_bytes: int
    host_free_disk_bytes: int


def preflight(config, required_gpu_mib=0):
    try:
        gpu = subprocess.run(['nvidia-smi', '--query-gpu=memory.total,memory.used,memory.free',
                              '--format=csv,noheader,nounits'], check=True, capture_output=True,
                             text=True, timeout=8).stdout.strip().splitlines()
        if len(gpu) != 1: raise ValueError('exactly one GPU required')
        total, used, free = (int(v.strip()) for v in gpu[0].split(','))
        if min(total, used, free) < 0 or total == 0 or used+free > total+2:
            raise ValueError('invalid GPU telemetry')
        mem = dict(line.split(':', 1) for line in Path('/proc/meminfo').read_text().splitlines())
        available = int(mem['MemAvailable'].split()[0]) * 1024
        guest_free = shutil.disk_usage('/').free
        host_free = shutil.disk_usage(config.host_system_path).free
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        raise ResourceError('resource telemetry unavailable; no job admitted') from error
    budget = min(config.gpu_ceiling_mib-used, free-config.spare_gpu_mib)
    snapshot = ResourceSnapshot(total, used, free, budget, available, guest_free, host_free)
    if budget <= 0 or budget < required_gpu_mib:
        raise ResourceError(f'insufficient measured GPU budget: {budget}MiB')
    if available < config.minimum_available_ram_bytes:
        raise ResourceError('insufficient available guest RAM')
    if min(guest_free, host_free) < config.minimum_free_disk_bytes:
        raise ResourceError('insufficient physical system disk headroom')
    return snapshot
