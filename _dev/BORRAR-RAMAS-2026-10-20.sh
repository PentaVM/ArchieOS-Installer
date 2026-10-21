#!/usr/bin/env bash
# Ramas verificadas como seguras de borrar (2026-10-20).
# Contenido ya contenido en otra rama o fusionado en main. Córrelo desde tu PC:
#   bash _dev/BORRAR-RAMAS-2026-10-20.sh
# o borra cada una en GitHub (Branches). NO borra feat/professional-gui-quickshell-vault ni main.
set -euo pipefail
R=origin

# Consolidadas en PR #46 (sentry/integrated):
git push $R --delete codex/sentry-discovery
git push $R --delete codex/sentry-host-wan-bridge
git push $R --delete codex/router-admin-library
git push $R --delete claude/sentry-ola3-cast-focos-servidores

# Contenida en PR #38 (codex/wanda-context):
git push $R --delete codex/wanda-strict-f32-proposal

# Fusionadas en main:
git push $R --delete claude/cargo-fmt-main
git push $R --delete design/orb-presets-cli
git push $R --delete feat/operator-kdenlive
git push $R --delete merge/xavi-gui
git push $R --delete todo/pendientes

echo "Listo. Conservadas: feat/professional-gui-quickshell-vault, main."
