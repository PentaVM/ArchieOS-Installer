#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
python=${PIPER_PYTHON:-python3}
"$python" -m pip install 'piper-tts==1.8.0' 'onnxruntime==1.30.0'
chmod +x "$repo_root/scripts/models/piper-json-bridge.py"
"$python" -c 'import onnxruntime, piper; print("Piper", piper.__file__, "ONNX Runtime", onnxruntime.__version__)'
