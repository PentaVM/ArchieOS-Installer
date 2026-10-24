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

# Archie's voices since 2026-10-24, chosen by measurement: each one read three sentences,
# and Nemotron ASR wrote back what it heard. es_MX-claude-high and en_US-kristin-medium
# were heard without a word wrong (ald: 2.8 %, john: 6.7 %) and speak in a third of real
# time. Claude's dataset is Apache-2.0; Kristin's recordings are LibriVox, public domain.
voices_revision=5041a7a49ae5b3a0f30e6837769eb03fdbb231bd
download_verified "$voices_revision" es/es_MX/claude/high/es_MX-claude-high.onnx   es_MX-claude-high.onnx cb1966e0ff20ca3aa010f6c9a0ce296a
download_verified "$voices_revision" es/es_MX/claude/high/es_MX-claude-high.onnx.json   es_MX-claude-high.onnx.json 36882201922fabb409f18d284af3eddd
download_verified "$voices_revision" es/es_MX/claude/high/MODEL_CARD   es_MX-claude-high.MODEL_CARD bca99dc9b9a6c8f2f8563d73d52c4a3b
download_verified "$voices_revision" en/en_US/kristin/medium/en_US-kristin-medium.onnx   en_US-kristin-medium.onnx 5fed42d2296baca042e2bf74785db725
download_verified "$voices_revision" en/en_US/kristin/medium/en_US-kristin-medium.onnx.json   en_US-kristin-medium.onnx.json 70bc97d350c796c64ea5e4d08241afac
download_verified "$voices_revision" en/en_US/kristin/medium/MODEL_CARD   en_US-kristin-medium.MODEL_CARD 9bfb8192299b34cddf76455f04cb8cd2

cat > "$voice_dir/archie-voice-attribution.txt" <<EOF
Archie voice pack, separate local Piper TTS models.

Spanish (Mexico), the default: es_MX-claude-high
  Source: https://huggingface.co/rhasspy/piper-voices/tree/$voices_revision/es/es_MX/claude/high
  Dataset: https://huggingface.co/spaces/HirCoir/Piper-TTS-Spanish; Apache-2.0 (see es_MX-claude-high.MODEL_CARD).
  Revision: $voices_revision

English (US), the default: en_US-kristin-medium
  Source: https://huggingface.co/rhasspy/piper-voices/tree/$voices_revision/en/en_US/kristin/medium
  Recorded speech dataset: LibriVox; public domain (see en_US-kristin-medium.MODEL_CARD).
  Revision: $voices_revision

English: en_US-john-medium
  Source: https://huggingface.co/rhasspy/piper-voices/tree/$en_revision/en/en_US/john/medium
  Recorded speech dataset: LibriVox; public domain (see en_US-john-medium.MODEL_CARD).
  Revision: $en_revision

Spanish (Mexico): es_MX-ald-medium
  Source: https://huggingface.co/rhasspy/piper-voices/tree/$es_revision/es/es_MX/ald/medium
  Recorded speech dataset: Unlicense (see es_MX-ald-medium.MODEL_CARD).
  Revision: $es_revision

All are openly licensed voices. They are not Baymax recordings, a Baymax voice clone,
or endorsed by Disney. Piper and each voice model have separate licenses.
EOF

printf 'Both voices are installed in %s\n' "$voice_dir"
