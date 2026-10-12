# Relevo vigente — 2026-10-11

**Actualización (Claude, 2026-10-11 noche):** `main` es la única rama y todo está
fusionado, incluido el prototipo de GUI de Xavi. Lo que falta está en
`../docs/PENDIENTES.md` y en el PR en borrador `todo/pendientes`.

**Actualización de voz e integración (Codex):** continuar exclusivamente en
`PentaVM/ArchieOS-Nvidia`. Leer primero
`../docs/HANDOFF-2026-10-11-SPEECH-FLOOR.md`. Main incorpora `feat/sentry-complete`
hasta `aaf2071`; el trabajo bilingüe pendiente se adapta sobre las implementaciones
actuales. No copiar versiones antiguas de memoria ni reaplicar commits equivalentes.
Claude/Xavi conservan el port WSL y la nueva GUI profesional; este trabajo no los ejecuta.
Los párrafos inferiores conservan contexto histórico, no destinos de trabajo vigentes.

**Actualización (Claude, mismo día):** el dueño decidió usar solo `PentaVM/ArchieOS-Nvidia`.
Sentry ya está portado aquí; `PentaVM/ArchieOS` no se continúa. Ver la entrada más
reciente de `BITACORA.md`.

Leer `../docs/HANDOFF-2026-10-11-SENTRY-CONTEXTO.md` y `BITACORA.md`.

Este checkout está en `main` de `PentaVM/ArchieOS-Nvidia`, actualizado con los merges
de Claude de reasoning/REM, `archie-lan` y Veil. La integración `18613ba` incorpora
`da3eb6f`: dominios de contexto, pilares y separación Enterprise personal/laboral.
No sustituye Core, Cortex/Cerebra ni IRIS.

Continuar aquí, no desde el antiguo `ArchieOS-nvidia-unify`. Este último conserva
un cambio de Cargo.lock que corresponde a dependencias ya presentes en upstream;
se dejó intacto. No copiar ese lock sobre main ni borrar sus archivos.

El otro main es `C:/Users/Penta/Desktop/Archie`, remoto `PentaVM/ArchieOS`.
Recibió IoT HA, análisis pasivo TShark y el port del módulo LAN. No son el mismo
árbol: el handoff compartido explica qué capacidades residen en cada uno.

Prioridad: voz, terminar Sentry/IoT/red defensiva y conectar eventos con los loops,
Vault/REM y el trabajo de Santiago. Revisar pendientes del módulo LAN antes de
desplegarlo; su manifiesto aún no equivale a un worker instalado.
La biometría real de voz y otras modalidades y la app iOS siguen pendientes.

El dueño autorizó commits, merge y push de este trabajo. Conservar los permisos
del producto y distinguir resultados reales de fixtures, hipótesis y código aún
sin productor conectado. Verificar el hash remoto antes de asumir publicación.
