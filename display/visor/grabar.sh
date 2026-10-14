#!/bin/bash
# Graba el grafo una sola vez: arranca el núcleo de Archie Lumenfield sin ventana sobre
# /srv/archieos, guarda todas sus respuestas en /srv/visor/snap (grabar.py) y lo detiene.
# Uso: LUMEN_CORE=/ruta/al/binario/archie bash grabar.sh
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
LUMEN_CORE="${LUMEN_CORE:-/srv/lf/core/target/release/archie}"
STATE=/srv/visor/lumen; mkdir -p "$STATE/cache" "$STATE/config"
rm -f "$STATE/url.txt" /srv/visor/base-ids.json /srv/visor/nuevos.json; rm -rf /srv/visor/snap
ARCHIE_INITIAL_ROOT=/srv/archieos ARCHIE_URL_FILE="$STATE/url.txt" ARCHIE_CACHE_DIR="$STATE/cache" \
  XDG_CONFIG_HOME="$STATE/config" HOME="$STATE" "$LUMEN_CORE" --headless > "$STATE/core.log" 2>&1 &
CORE=$!
for i in $(seq 1 120); do [ -s "$STATE/url.txt" ] && break; sleep 1; done
LUMEN_URLFILE="$STATE/url.txt" python3 "$DIR/grabar.py" | tail -4
kill $CORE
du -sh /srv/visor/snap
