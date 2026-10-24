# Decisiones provisionales de la alpha 0.1 — 2026-10-23

El dueño pidió seguir sin detenerse ("continúa completo en loop") antes de responder las
decisiones de la sección 3 de `Desktop/ArchieOS/AUDITORIA-0.1-2026-10-23.md`. Estas son
las opciones que se tomaron para no bloquear el trabajo. Todas son reversibles. Si el
dueño responde distinto, esta tabla se actualiza y el trabajo afectado se rehace.

| # | Tema | Decisión provisional | Por qué |
|---|---|---|---|
| 1 | SSIM del port | Se mide con `archie-paint::bench::ssim` (luma, ventanas 8×8, paso 4) sobre el recorte del grafo de `compare-motor-visuals.mjs` (270,85, 1005×755), con capturas deterministas (semilla fija, tiempo congelado). Se informa por preset. La meta es ≥ 0,98; si un preset no llega, se informa el número tal cual. | Ya existe en el repo y es reproducible. |
| 2 | Texturas | Mapas de color (nodos y carpetas): generador exacto desde la baldosa de 1024 (espejo y repetición), con sha256 de los píxeles decodificados. Mapas de superficie: generador con la regla de espejo ±1, mismo control. Mapa de energía: queda como PNG (no es periódico; un generador sería la imagen embebida). Mapas prisma: quedan los PNG actuales. | Lo exacto donde es posible, sin inventar un generador que solo copia datos. |
| 3 | Tipografía | Inter en todo el texto de interfaz. Space Grotesk se mantiene solo en títulos de display, como en el diseño de Xavi. | Respeta el diseño y la regla "fuente Inter". |
| 4 | Hermes | Fuera de la 0.1 como runner de inferencia. Queda como sonda (`archie harness test hermes`). | No tiene adaptador de inferencia ni API de texto. |
| 5 | Codex en el OVA | No se incluye ni se preconfigura. El OVA nunca lleva credenciales. Codex sigue funcionando desde Windows nativo. | La sesión de Codex es del dueño. |
| 6 | Pesos | El disco del OVA lleva Bonsai 2 9B (Apache-2.0) y el checkpoint de Wanda LNCM. No lleva los pesos de IRIS (licencia no comercial, `docs/IRIS-LICENSES.md`). La ISO no lleva pesos. | Licencias. |
| 7 | Botones deshabilitados | Se implementan los que tienen un verbo en el backend; los demás se ocultan. Ningún botón visible queda sin efecto. | "Todos los botones funcionan". |
| 8 | Evaluador confiable | No hay en la 0.1. La promoción de Harness² sigue denegada por defecto. | Sin evaluador, una promoción no prueba mejora. |
| 9 | Interop de la WSL | Sigue desactivado (`wsl.conf`, `SECURITY-BOUNDARY.md`). El turno real de Codex corre en Windows nativo. | Frontera de seguridad. |
| 10 | `AGENTS.md` del PR 50 | Se acepta tal como quedó fusionado (dos autores: Xavi diseño, PentaVM implementación nativa). | Ya está en el tronco. |
| 11 | Hugging Face | Repositorio privado nuevo `PentaVM/ArchieOS-0.1`. No se toca `PentaVM/ArchieOS-Bonsai2-9B-WSL`. | El dueño pidió privado. |
| 12 | Autoría | Los 2 commits del PR 49 con autor "Claude" quedan como están (no hay push forzado). Todo lo nuevo va como PentaVM. | Reescribir exige forzar el push. |
| 13 | Ramas | No se borra ninguna rama. El tronco es `claude/unified-0.1`; el port avanza en `claude/lumenfield-port` y se fusiona al tronco. | Borrar es decisión del dueño. |
| 14 | GPU | Se usa para Bonsai (harness y verificación). No se relanza el entrenamiento de Wanda. | Un solo turno de GPU. |

## Estado de las ramas al empezar (2026-10-23 07:40)

- `claude/unified-0.1` = `1d7f1c2` → avance rápido a `d5bf2c8` (PR 50) → fusión del PR 49
  (`c81bf3c`) en `5d63ced`, sin conflictos.
- `claude/lumenfield-port` avanzó a `5d63ced`. Su registro
  `_dev/LUMENFIELD-PORT-2026-10-22.md` sigue sin commit en ese worktree.
- Las sesiones de unificación, harness JSON y port no tenían actividad desde hacía más de
  90 minutos ni procesos de compilación; esta sesión toma los tres frentes.
