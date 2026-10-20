#!/bin/bash
# Deja listo el visor (una sola vez, como root en Arch):
#  1. el contenido del sistema: este repo, solo lo versionado (git archive, sin .git ni compilados),
#     en /srv/archieos;
#  2. el motor de la esfera: la web compilada de Archie Lumenfield 0449 y sus texturas, en web/.
# Uso: LUMEN_WEB=/ruta/a/web/dist LUMEN_TEXTURES=/ruta/a/web/art/textures bash preparar.sh [commit]
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$DIR/../.." && pwd)"
COMMIT="${1:-HEAD}"
: "${LUMEN_WEB:?indica LUMEN_WEB: la carpeta dist de la web de Archie Lumenfield 0449}"
: "${LUMEN_TEXTURES:?indica LUMEN_TEXTURES: la carpeta art/textures de Archie Lumenfield}"
rm -rf /srv/archieos && mkdir -p /srv/archieos /srv/visor
git -C "$REPO" archive "$COMMIT" | tar -x -C /srv/archieos
echo "contenido: $(find /srv/archieos -type f | wc -l) archivos, $(du -sh /srv/archieos | cut -f1)"
rm -rf "$DIR/web" && mkdir -p "$DIR/web/textures"
cp -r "$LUMEN_WEB/." "$DIR/web/"
cp -rn "$LUMEN_TEXTURES/." "$DIR/web/textures/"
rm -f /srv/visor/base-ids.json /srv/visor/nuevos.json
echo "motor: $(du -sh "$DIR/web" | cut -f1) en $DIR/web"
