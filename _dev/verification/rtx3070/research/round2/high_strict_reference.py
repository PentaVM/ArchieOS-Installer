"""One bounded numerical experiment; never starts a training worker or edits its controls."""
import argparse
import dataclasses
import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO / 'training/wanda_bonsai2/src'))
from archie_wanda.contracts import TrainConfig
from archie_wanda.native import sha_file
from archie_wanda.resources import preflight


def configuration(root):
    return TrainConfig(model_path='/home/archie/wanda/models/Ternary-Bonsai-2-9B-PTQ1_0.gguf',
                       run_dir=str(root), host_system_path='/mnt/c', sequence_length=128)


def thermal_sample():
    result = subprocess.run(['/usr/lib/wsl/lib/nvidia-smi',
                             '--query-gpu=temperature.gpu,memory.used,power.draw',
                             '--format=csv,noheader,nounits'], check=True, capture_output=True,
                            text=True, timeout=3)
    lines = result.stdout.strip().splitlines()
    if len(lines) != 1:
        raise RuntimeError('single GPU telemetry required')
    temp, used, power = map(float, lines[0].split(','))
    return {'elapsed': time.monotonic(), 'temperature_c': temp, 'used_gpu_mib': used, 'power_w': power}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', required=True)
    parser.add_argument('--stage', choices=('guard', 'native', 'torch', 'backward'), default='guard')
    parser.add_argument('--experiment', choices=('reference', 'backward'), default='reference')
    args = parser.parse_args()
    root = Path(args.root)
    config = configuration(root)
    out = Path(__file__).resolve().parent
    if args.stage == 'native':
        from archie_wanda.native import verify_extraction
        verify_extraction(config,
            '/mnt/c/Users/Penta/Archie-RTX3070-Handoff/local-verification/prism-b10777-source',
            REPO / 'training/wanda_bonsai2/tests/hardware/public-parity.jsonl',
            out / 'strict-extraction.json', 'strict-f32')
        return
    if args.stage == 'torch':
        from archie_wanda.verification import verify_suffix
        verify_suffix(config, out / 'strict-extraction.json', out / 'strict-suffix.json')
        return
    if args.stage == 'backward':
        from archie_wanda.verification import verify_loss
        verify_loss(config, out / 'strict-suffix.json', out / 'strict-backward.json')
        return
    root.mkdir(parents=True, exist_ok=False)
    (root / 'config.json').write_text(json.dumps(dataclasses.asdict(config), indent=2))
    telemetry = {'format_version': 1, 'scope': 'owned_short_numerical_reference_only',
                 'deadline_seconds': 180, 'start_below_c': 60, 'stop_at_c': 70,
                 'semantic_training': False, 'samples': [], 'phases': [],
                 'script_sha256': sha_file(__file__), 'started_unix': time.time()}
    owned = None
    started = time.monotonic()
    with open('/home/archie/wanda/run/worker.lock', 'r') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        worker = json.loads(Path('/home/archie/wanda/run/worker.json').read_text())
        if worker['state'] != 'stopped':
            raise RuntimeError('existing worker is not stopped')
        telemetry['original_worker'] = worker
        try:
            first = thermal_sample()
            telemetry['samples'].append(first)
            if first['temperature_c'] >= 60:
                raise RuntimeError('GPU too warm for authorized experiment')
            preflight(config)
            for stage in (('native', 'torch') if args.experiment == 'reference' else ('backward',)):
                # Waiting for the previous process to exit frees its CUDA context before the next phase.
                with (root / (stage + '.log')).open('w') as log:
                    owned = subprocess.Popen([sys.executable, __file__, '--root', str(root), '--stage', stage],
                                             stdout=log, stderr=log, start_new_session=True)
                    telemetry['phases'].append({'phase': stage, 'owned_pid': owned.pid})
                    while owned.poll() is None:
                        sample = thermal_sample()
                        sample['elapsed'] -= started
                        telemetry['samples'].append(sample)
                        preflight(config)
                        if sample['temperature_c'] >= 70 or sample['used_gpu_mib'] > 7168:
                            raise RuntimeError('owned experiment exceeded temperature/memory stop threshold')
                        if time.monotonic() - started >= 180:
                            raise RuntimeError('owned experiment exceeded 180-second deadline')
                        time.sleep(.25)
                    telemetry['phases'][-1]['exit_code'] = owned.returncode
                    if owned.returncode:
                        raise RuntimeError('numerical phase failed: ' + stage + '; see ' + str(root / (stage + '.log')))
                    owned = None
            telemetry['status'] = 'passed'
        except BaseException as error:
            telemetry['status'] = 'failed'
            telemetry['error'] = str(error)
            if owned is not None and owned.poll() is None:
                os.killpg(owned.pid, signal.SIGTERM)
                try:
                    owned.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(owned.pid, signal.SIGKILL)
                    owned.wait(timeout=3)
            raise
        finally:
            telemetry['seconds'] = time.monotonic() - started
            guard_name = 'strict-guard.json' if args.experiment == 'reference' else 'strict-backward-guard.json'
            (out / guard_name).write_text(json.dumps(telemetry, indent=2) + '\n')
    print(json.dumps({'status': telemetry['status'], 'seconds': telemetry['seconds'],
                      'max_temperature_c': max(x['temperature_c'] for x in telemetry['samples']),
                      'max_gpu_mib': max(x['used_gpu_mib'] for x in telemetry['samples'])}))


if __name__ == '__main__':
    main()
