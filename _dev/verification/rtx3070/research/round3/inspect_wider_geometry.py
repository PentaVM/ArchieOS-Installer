"""CPU-only read-only GGUF directory evidence; never loads a tensor or tokenizer."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import sys

REPO = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO / 'training/wanda_bonsai2/src'))
from archie_wanda.model import inspect_model
from archie_wanda.contracts import BASE_SHA


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--model', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    model = inspect_model(args.model, BASE_SHA)
    relevant = {k:v for k,v in model.fields.items()
                if k == 'general.architecture' or
                (k.startswith('qwen35.') and not k.startswith('tokenizer.'))}
    tensors = {name: {'shape': list(t.shape), 'ggml_type': t.kind}
               for name,t in model.tensors.items()
               if any(name.startswith(f'blk.{i}.') for i in range(28, 32))}
    assert 'torch' not in sys.modules
    result = {'observed_utc': datetime.now(timezone.utc).isoformat(),
              'scope': 'CPU file SHA256, metadata and tensor-directory inspection only',
              'base_sha256': model.sha256, 'file_bytes': model.size,
              'metadata': relevant, 'tensors_blocks_28_31': tensors,
              'tensor_payloads_loaded': False, 'tokenizer_loaded': False,
              'torch_imported': False, 'gpu_used': False}
    args.out.write_bytes((json.dumps(result, indent=2) + '\n').encode('utf-8'))
    print(json.dumps(result))
