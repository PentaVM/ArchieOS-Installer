#!/usr/bin/env bash
# Builds the self-contained ArchieOS WSL image the Windows launcher installs (the signed
# manifest's "operator" artifact): the Operator root that scripts/wsl/build-operator.sh
# assembles, plus what it needs to hear and speak with nothing downloaded: NVIDIA Nemotron 3.5
# ASR, the Spanish and English Piper voices and their Python runtime. IRIS is already in the
# Operator root. The reasoning model is not: the person chooses it in the launcher, which
# fetches it with archie-local, so nobody downloads a model only to delete it.
#
#   ARCHIE_WSL_ROOTFS_ONLY=1 ARCHIE_WSL_WORK_DIR=/var/tmp/archieos-wsl-offline \
#     IRIS_WEIGHTS=... (as for build-operator.sh) scripts/wsl/build-operator.sh
#   ASR=/path/nemotron-3.5-asr-streaming-0.6b.q8_0.hf.gguf \
#   PIPER_VOICES=/var/lib/archie/models/piper SOURCE_SHA=<40-hex commit> \
#     scripts/wsl/build-offline.sh /var/tmp/archieos-wsl-offline/rootfs OUT.wsl
#
# The image gets the offline first start (archie-offline-oobe) and the payload manifest
# archie-offline --check verifies; scripts/wsl/package-offline.sh refuses anything that is not
# clean, and writes OUT.wsl with its SHA-256. Run as root on Arch Linux.
set -euo pipefail
rootfs=$(realpath "${1:?usage: build-offline.sh OPERATOR_ROOTFS OUT.wsl}")
output=$(realpath -m "${2:?usage: build-offline.sh OPERATOR_ROOTFS OUT.wsl}")
repo_root=$(realpath "$(dirname "$0")/../..")
[[ $EUID -eq 0 ]] || { echo 'run as root on Arch Linux' >&2; exit 1; }
[[ -x $rootfs/usr/bin/archied && -f $rootfs/var/lib/archie/models/iris/iris-beta21-step132000.pt ]] || {
  echo "build-offline: $rootfs is not an assembled Operator root (ARCHIE_WSL_ROOTFS_ONLY=1 build-operator.sh)" >&2; exit 66; }
# NVIDIA's Nemotron 3.5 streaming ASR, the GGUF that embeds its tokenizer (word boosting works).
asr=${ASR:?ASR is nemotron-3.5-asr-streaming-0.6b.q8_0.hf.gguf}
voices=${PIPER_VOICES:?PIPER_VOICES is the folder install-piper-voices.sh filled}
source_sha=${SOURCE_SHA:?SOURCE_SHA is the commit the image is built from}
[[ $source_sha =~ ^[0-9a-f]{40}$ ]] || { echo "build-offline: SOURCE_SHA must be a full commit id" >&2; exit 64; }
for voice in es_MX-ald-medium en_US-john-medium; do
  [[ -f $voices/$voice.onnx && -f $voices/$voice.onnx.json ]] || {
    echo "build-offline: $voice is not in $voices (scripts/models/install-piper-voices.sh)" >&2; exit 66; }
done

models=$rootfs/var/lib/archie/models
install -Dm 0644 "$asr" "$models/asr/$(basename "$asr")"
printf '%s\n' 'NVIDIA Nemotron 3.5 ASR streaming 0.6B, q8_0 GGUF, under the NVIDIA Open Model License.' \
  'https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b' > "$models/asr/SOURCE"
for voice in es_MX-ald-medium en_US-john-medium; do
  install -m 0644 "$voices/$voice.onnx" "$voices/$voice.onnx.json" "$models/piper/"
  [[ -f $voices/$voice.MODEL_CARD ]] && install -m 0644 "$voices/$voice.MODEL_CARD" "$models/piper/"
done
chown -R 1000:1000 "$models/asr" "$models/piper"

# The voices' runtime, pinned as archie-setup installs it, so the first start needs no network.
# The wheels are fetched here (the root has no resolver) and installed inside without an index.
wheels=$rootfs/tmp/voice-wheels
rm -rf "$wheels" && install -d "$wheels"
python3 -m venv "$wheels.venv"
"$wheels.venv/bin/pip" download --quiet --dest "$wheels" "piper-tts==1.8.0" "onnxruntime==1.30.0"
rm -rf "$wheels.venv"
arch-chroot "$rootfs" bash -c 'rm -rf /opt/archie/voice-venv && python -m venv /opt/archie/voice-venv \
  && /opt/archie/voice-venv/bin/pip install --quiet --no-index --find-links /tmp/voice-wheels "piper-tts==1.8.0" "onnxruntime==1.30.0" \
  && /opt/archie/voice-venv/bin/python -c "import onnxruntime, piper"'
rm -rf "$wheels"
chown -R 1000:1000 "$rootfs/opt/archie/voice-venv"

# The offline first start: a machine-local package key, the payload check, the desktop.
cat > "$rootfs/etc/wsl-distribution.conf" <<'EOF'
[oobe]
command=/usr/lib/wsl/archie-offline-oobe
defaultUid=1000
defaultName=ArchieOS

[shortcut]
enabled=true

[windowsterminal]
enabled=true
EOF
chmod 0755 "$rootfs/usr/lib/wsl/archie-offline-oobe" "$rootfs/usr/bin/archie-offline"

# What archie-offline --check verifies: each model's path, size and SHA-256, and the commit.
python3 - "$rootfs" "$source_sha" "$(basename "$asr")" <<'EOF'
import hashlib, json, pathlib, sys
root, source_sha, asr = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
files = {
    'asr': '/var/lib/archie/models/asr/' + asr,
    'piper_es': '/var/lib/archie/models/piper/es_MX-ald-medium.onnx',
    'piper_en': '/var/lib/archie/models/piper/en_US-john-medium.onnx',
    'iris': '/var/lib/archie/models/iris/iris-beta21-step132000.pt',
}
record = {}
for role, path in files.items():
    target = root / path.lstrip('/')
    with target.open('rb') as data:
        digest = hashlib.file_digest(data, 'sha256').hexdigest()
    record[role] = {'path': path, 'bytes': target.stat().st_size, 'sha256': digest}
out = root / 'usr/share/archie/offline/payload.json'
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps({'format': 1, 'source_sha': source_sha, 'files': record}, indent=2) + '\n')
print('payload:', ', '.join(f'{k} {v["bytes"]} B' for k, v in record.items()))
EOF
printf '%s\n' 'ArchieOS. Say "Archie" to talk. archie-offline starts the desktop; archie-talk, archie-operate, archie-video, archie-design.' > "$rootfs/etc/motd"
rm -rf "$rootfs/root/.cache" "$rootfs/var/log"/* "$rootfs/tmp"/*
du -sh "$rootfs"
bash "$repo_root/scripts/wsl/package-offline.sh" "$rootfs" "$output"
