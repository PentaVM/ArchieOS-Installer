#!/bin/bash
# Arranca el visor (servidor del grafo guardado) si no está en marcha. Sin núcleo Rust.
DIR="$(cd "$(dirname "$0")" && pwd)"
mkdir -p /srv/visor
pgrep -f "python3 $DIR/servidor.py" >/dev/null && exit 0
rm -f /srv/visor/url.txt
nohup setsid python3 "$DIR/servidor.py" > /srv/visor/servidor.log 2>&1 < /dev/null &
for i in $(seq 1 30); do [ -s /srv/visor/url.txt ] && break; sleep 0.5; done
