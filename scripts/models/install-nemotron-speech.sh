#!/usr/bin/env bash
set -euo pipefail

runtime_commit=a5b6953c4a579a2bbd1c0913ad8a85c2a4d99953
model_sha256=d9a01898d2a611c8764e23a1c2f45e70bbd5a425dc4de93692ac951dd603812d
model_name=nemotron-speech-streaming-en-0.6b.q8_0.gguf
model_url="https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b/resolve/main/$model_name"

if [[ $EUID -eq 0 ]]; then
  prefix=${ARCHIE_NEMO_PREFIX:-/opt/archie/nemo-speech}
  model_dir=${ARCHIE_MODEL_DIR:-/archie/models}
  link_dir=/usr/local/bin
else
  prefix=${ARCHIE_NEMO_PREFIX:-$HOME/.local}
  model_dir=${ARCHIE_MODEL_DIR:-$HOME/.local/share/archie/models}
  link_dir=$HOME/.local/bin
fi
install -d "$prefix" "$model_dir" "$link_dir"

installer=$(mktemp)
trap 'rm -f "$installer" "$model_dir/$model_name.part"' EXIT
curl --fail --location --proto '=https' --tlsv1.2 \
  "https://raw.githubusercontent.com/NVIDIA/NeMo-Speech.cpp/$runtime_commit/scripts/install.sh" \
  -o "$installer"
NEMO_SPEECH_SOURCE_REF="$runtime_commit" sh "$installer" \
  --source --backend auto --prefix "$prefix" --no-modify-path

curl --fail --location --proto '=https' --tlsv1.2 "$model_url" \
  -o "$model_dir/$model_name.part"
printf '%s  %s\n' "$model_sha256" "$model_dir/$model_name.part" | sha256sum --check --status
mv "$model_dir/$model_name.part" "$model_dir/$model_name"
ln -sfn "$prefix/bin/nemo-speech" "$link_dir/nemo-speech"
printf 'runtime_commit=%s\nmodel_sha256=%s\n' "$runtime_commit" "$model_sha256" \
  > "$model_dir/nemotron-speech-install.metadata"
echo "Nemotron Speech installed locally at $model_dir/$model_name"

