"""CPU-only numerical admission and resume identity; no CUDA imports or allocations."""
from dataclasses import asdict
import json
import math
import os
from pathlib import Path
from .contracts import BASE_SHA, PRISM_COMMIT, AdmissionError, digest
from .native import STRICT_F32_ENV, sha_file, judge_native_comparisons
from .contracts import ParityError


def _receipt(spec, key):
    path = Path(spec.get(key, ''))
    if not path.is_file() or path.stat().st_size > 1024 * 1024:
        raise AdmissionError('missing/bounded numerical gate required: ' + key)
    return json.loads(path.read_text()), sha_file(path)


def validate_gates(config, spec):
    if sorted(spec.get('targets', ['q', 'v'])) != ['q', 'v']:
        raise AdmissionError('current strict backward proof admits q/v only')
    if spec.get('precision_profile') != 'strict-f32':
        raise AdmissionError('training requires an explicitly declared strict-f32 profile')
    extraction, extraction_sha = _receipt(spec, 'extraction_receipt')
    parity, parity_sha = _receipt(spec, 'parity_receipt')
    backward, backward_sha = _receipt(spec, 'backward_receipt')
    if (extraction.get('precision_profile') != 'strict-f32' or
            extraction.get('precision_environment') != STRICT_F32_ENV or
            extraction.get('runtime_commit') != PRISM_COMMIT or
            extraction.get('base_sha256') != BASE_SHA or
            extraction.get('scope') != 'real_native_full_row_capture_microbatch_reset' or
            parity.get('status') != 'passed' or parity.get('scope') != 'real_zero_adapter_suffix' or
            parity.get('base_sha256') != BASE_SHA or parity.get('precision_profile') != 'strict-f32' or
            parity.get('extraction_sha256') != extraction_sha or len(parity.get('cases', [])) < 3 or
            backward.get('status') != 'passed' or
            backward.get('scope') != 'real_suffix_full_vocabulary_backward' or
            backward.get('base_sha256') != BASE_SHA or backward.get('precision_profile') != 'strict-f32' or
            backward.get('parity_sha256') != parity_sha):
        raise AdmissionError('strict extraction/parity/backward receipt chain mismatch')
    extraction_cases = extraction.get('cases', [])
    if (len(extraction_cases) < 3 or len({case.get('id') for case in extraction_cases}) != len(extraction_cases) or
            any(not isinstance(case.get('id'), str) or not case['id'] or
                type(case.get('tokens')) is not int or case['tokens'] < 10 for case in extraction_cases)):
        raise AdmissionError('three complete strict native extraction cases are required')
    try:
        for case in extraction_cases:
            judge_native_comparisons(case.get('comparisons', {}), 'strict-f32')
    except (ParityError, KeyError, TypeError) as error:
        raise AdmissionError('invalid strict native extraction measurements') from error
    if ({(case.get('id'), case.get('tokens')) for case in parity['cases']} !=
            {(case['id'], case['tokens']) for case in extraction_cases} or
            len(parity['cases']) != len(extraction_cases) or
            parity.get('torch_float32_matmul_precision') != 'highest'):
        raise AdmissionError('strict suffix cases/precision do not match extraction')
    for case in parity['cases']:
        rms, kl = case.get('normalized_relative_rms'), case.get('maximum_token_kl')
        if (case.get('criterion') != 'strict' or type(rms) not in (int, float) or
                type(kl) not in (int, float) or not math.isfinite(rms) or not math.isfinite(kl) or
                not 0 <= rms <= 5e-4 or not 0 <= kl <= 1e-3):
            raise AdmissionError('original suffix tolerance is required; no layout-noise admission')
    steps = backward.get('steps', [])
    if (backward.get('vocabulary') != 248320 or type(backward.get('tokens')) is not int or
            backward['tokens'] < 2 or backward.get('torch_float32_matmul_precision') != 'highest' or
            len(steps) != 2 or [step.get('step') for step in steps] != [1, 2]):
        raise AdmissionError('two measured full-vocabulary highest-FP32 backward steps are required')
    gradient_names = {'q_delta.a','q_delta.b','v_delta.a','v_delta.b'}
    for number, step in enumerate(steps):
        loss = step.get('loss')
        gradients = step.get('gradient_norms', {})
        if (type(loss) not in (int, float) or not math.isfinite(loss) or loss < 0 or
                set(gradients) != gradient_names or
                any(type(value) not in (int, float) or not math.isfinite(value) or value < 0 or
                    (number == 1 and value == 0) for value in gradients.values())):
            raise AdmissionError('backward loss/complete A/B gradients must be finite; step2 gradients nonzero')
    runtimes = extraction.get('runtime_library_sha256', {})
    required = {'libllama.so', 'libggml-cuda.so', 'libggml-base.so'}
    if set(runtimes) != required or any(sha_file(Path(config.runtime_dir) / name) != runtimes[name] for name in required):
        raise AdmissionError('runtime changed since strict reference')
    return {'base_sha256': BASE_SHA, 'precision_profile': 'strict-f32',
            'precision_environment': dict(STRICT_F32_ENV), 'runtime_commit': PRISM_COMMIT,
            'shim_sha256': extraction.get('shim_sha256'),
            'runtime_library_sha256': runtimes, 'extraction_sha256': extraction_sha,
            'parity_sha256': parity_sha, 'backward_sha256': backward_sha}


def validate_cache_profile(cache, admission):
    if (cache.get('precision_profile') != admission['precision_profile'] or
            cache.get('precision_environment') != admission.get('precision_environment') or
            cache.get('runtime_commit') != admission.get('runtime_commit') or
            cache.get('runtime_library_sha256') != admission.get('runtime_library_sha256') or
            cache.get('base_sha256') != admission.get('base_sha256')):
        raise AdmissionError('training cache does not match the admitted numerical profile')
    if not admission.get('shim_sha256') or cache.get('shim_sha256') != admission['shim_sha256']:
        raise AdmissionError('training cache shim differs from the strict reference')


def configure_training_precision(torch):
    """Pin effective process policy before allocation; machine-wide settings are untouched."""
    for key in ('TORCH_ALLOW_TF32_CUBLAS_OVERRIDE', 'NVIDIA_TF32_OVERRIDE'):
        if os.environ.get(key) not in (None, '0'):
            raise AdmissionError('conflicting training precision environment: ' + key)
    os.environ['NVIDIA_TF32_OVERRIDE'] = '0'
    torch.set_float32_matmul_precision('highest')
    torch.backends.cuda.matmul.allow_tf32 = False
    policy = {'float32_matmul_precision': torch.get_float32_matmul_precision(),
              'allow_tf32': torch.backends.cuda.matmul.allow_tf32,
              'NVIDIA_TF32_OVERRIDE': os.environ['NVIDIA_TF32_OVERRIDE'],
              'TORCH_ALLOW_TF32_CUBLAS_OVERRIDE': os.environ.get('TORCH_ALLOW_TF32_CUBLAS_OVERRIDE')}
    if policy['float32_matmul_precision'] != 'highest' or policy['allow_tf32']:
        raise AdmissionError('effective training policy does not match the strict FP32 reference')
    return policy


def validate_lora_keys(state, targets):
    expected = {target + '_delta.' + matrix for target in targets for matrix in ('a', 'b')}
    if set(state) != expected:
        raise AdmissionError('checkpoint must hold exactly the complete selected LoRA A/B pairs')


def run_identity(config, settings, examples, dataset_paths, admission):
    sources = ('admission.py', 'worker.py', 'suffix.py', 'ptq.py', 'head_loss.py', 'model.py', 'data.py')
    return digest({'settings': settings, 'config': asdict(config), 'admission': admission,
                   'init_checkpoint_sha256':sha_file(settings['init_from']) if settings.get('init_from') else None,
                   'examples': examples,
                   'datasets': [(str(path), sha_file(path)) for path in dataset_paths],
                   'cache_receipts': [(e['cache'], sha_file(Path(e['cache']) / 'receipt.json'))
                                      for e in examples if (Path(e['cache']) / 'receipt.json').is_file()],
                   'implementation': {name: sha_file(Path(__file__).with_name(name)) for name in sources}})
