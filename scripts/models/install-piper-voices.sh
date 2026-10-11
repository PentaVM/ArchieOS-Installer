#!/usr/bin/env bash
set -euo pipefail

# Open, locale-matched Piper voices for Archie. Model files stay outside the repository;
# each model card is installed beside its weights so the speaker's dataset terms travel
# with the voice.
download_verified() {
  local revision=$1 remote_path=$2 filename=$3 expected_md5=$4
  local destination="$voice_dir/$filename"
  local partial="$destination.part.$$"
  if [[ -f $destination ]] && printf '%s  %s\n' "$expected_md5" "$destination" | md5sum --check --status; then
    printf 'Already verified: %s\n' "$filename"
    return
  fi
  curl --fail --location --proto '=https' --tlsv1.2 \
    "https://huggingface.co/rhasspy/piper-voices/resolve/$revision/$remote_path" \
    -o "$partial"
  if ! printf '%s  %s\n' "$expected_md5" "$partial" | md5sum --check --status; then
    rm -f "$partial"
    echo "Checksum mismatch: $filename" >&2
    return 1
  fi
  mv "$partial" "$destination"
  printf 'Installed and verified: %s\n' "$filename"
}

repo_root=$(cd "$(dirname "$0")/../.." && pwd)
voice_dir=${PIPER_VOICES:-$repo_root/models-cache/piper}
mkdir -p "$voice_dir"
chmod +x "$repo_root/scripts/models/piper-json-bridge.py"

# The English recording dataset is LibriVox public domain. Its Piper model card carries
# the recording source and training details.
en_revision=e70533d22ac02da609b7427dd1a8bd5349295e98
download_verified "$en_revision" en/en_US/john/medium/en_US-john-medium.onnx \
  en_US-john-medium.onnx 70480857f21f2560f3a232722023b36d
download_verified "$en_revision" en/en_US/john/medium/en_US-john-medium.onnx.json \
  en_US-john-medium.onnx.json f2d04611b498e14d394385d1ec8a2d2d
download_verified "$en_revision" en/en_US/john/medium/MODEL_CARD \
  en_US-john-medium.MODEL_CARD 4ef938585cf2cc8da4ada9b6d2c579ec

# The Spanish recordings are published under Unlicense.
es_revision=2f8dbe0bb0dde986411632bf014a13cdbe6596e7
download_verified "$es_revision" es/es_MX/ald/medium/es_MX-ald-medium.onnx \
  es_MX-ald-medium.onnx 86374058e59b41ac3b7fe4181e1daad6
download_verified "$es_revision" es/es_MX/ald/medium/es_MX-ald-medium.onnx.json \
  es_MX-ald-medium.onnx.json c17db4ddbef38305701e4047a3adbcf3
download_verified "$es_revision" es/es_MX/ald/medium/MODEL_CARD \
  es_MX-ald-medium.MODEL_CARD a858af3698e0c7cda6c9ad5d0d11b651

cat > "$voice_dir/archie-voice-attribution.txt" <<EOF
Archie voice pack, two separate local Piper TTS models.

English: en_US-john-medium
  Source: https://huggingface.co/rhasspy/piper-voices/tree/$en_revision/en/en_US/john/medium
  Recorded speech dataset: LibriVox; public domain (see en_US-john-medium.MODEL_CARD).
  Revision: $en_revision

Spanish (Mexico): es_MX-ald-medium
  Source: https://huggingface.co/rhasspy/piper-voices/tree/$es_revision/es/es_MX/ald/medium
  Recorded speech dataset: Unlicense (see es_MX-ald-medium.MODEL_CARD).
  Revision: $es_revision

The English voice is an openly licensed US English male voice and the Spanish voice is
an openly licensed Mexican Spanish voice. They are not Baymax recordings, a Baymax voice
clone, or endorsed by Disney. Piper and each voice model have separate licenses.
EOF

printf 'Both voices are installed in %s\n' "$voice_dir"
