# Pendientes — ArchieOS

Estado al 2026-10-11. `main` es la única rama; todo lo hecho hasta hoy está fusionado.
Esta lista es lo que falta para llamar terminado a cada parte. Cada punto dice quién lo
tiene y dónde está el detalle. Se marca aquí cuando se cierra.

## 1. Release y WSL

- [ ] **Imagen WSL desde `main`.** La pre-release `wsl-operator-2026.10.11` salió de
  `b22625e`: no tiene la voz (`e5cf04a`), ni Kdenlive, ni el bridge de video. El
  `build-operator.sh` de `main` ya los pide; falta construirla. Bloqueo: el portátil
  constructor tiene 2 GB libres y hacen falta unos 8. — Claude
- [ ] **Probar IRIS desde la imagen en una GPU.** Nunca se ejecutó: el constructor no
  tiene GPU y con 7 GB de RAM el modelo no carga ni en CPU. Primer dato real: la GTX 1660
  del amigo de Nelson (float32). Anotar latencia por decisión y memoria. — Nelson
- [ ] **Probar `install-archie-operator.ps1` en una máquina limpia.** No se importó la
  imagen en ningún equipo. — Sheriff / Nelson
- [ ] **Kdenlive con Sheriff.** Dos caminos, ninguno probado en la imagen:
  bridge de video (`archie-bridges call`, MLT sin ventana) y `archie-operate --app kdenlive`
  (IRIS en el Stage). El E2E del bridge existe (`ARCHIE_BRIDGE_E2E=video`). — Sheriff
- [ ] **ISO.** `distro/archiso/build.sh` incluye `archie-lan`, `archie-netwatch`,
  `archie-rem`, `archie-print` y `archie-ask`; no se ha construido ni arrancado desde
  esos cambios. CI sigue apagado por facturación. — sin dueño
- [ ] **Licencia de IRIS para distribuir.** Los pesos son de investigación no comercial;
  el release es privado. Decidir antes de cualquier publicación abierta. — Nelson

## 2. IRIS

- [ ] **Runtime sin Python.** `iris_serve` es PyTorch; la regla del proyecto es C++/Rust.
  Exportar a ONNX o escribir el runtime en Rust. Mientras tanto la imagen instala
  PyTorch en el primer arranque (3 GB) y baja SigLIP (1,5 GB). — Claude
- [ ] **Adaptadores LoRA por aplicación**, en los nodos de Sheriff y Xavi. — Nelson
- [ ] **REM no graba demostraciones de una persona**, solo corridas verificadas de Arena.
- [ ] **Medir IRIS en 6–8 GB.** El manifiesto dice 56 ms en una RTX 5090 que además
  entrenaba; no hay medida en una GPU propia.
- [ ] Fusionar `PentaVM/archie-iris#27` (float32 en GPU sin bf16 nativo, `einops`).

## 3. Sentry

Detalle y defectos conocidos: [`HANDOFF-2026-10-11-SENTRY-AUDITORIA.md`](HANDOFF-2026-10-11-SENTRY-AUDITORIA.md).
Kimi K3 toma el siguiente avance en un PR aparte.

- [ ] **Una sola vista de dispositivos.** `archie-lan` (inventario), `archie-netwatch`
  (tráfico) y `passive_network` (importador) son tres fuentes con tres formatos; ninguna
  llega al world graph.
- [ ] **Alertas → `PillarContext` → Cerebra.** Nada produce un `PillarContext` todavía.
- [ ] **Respuesta.** Todo es observación: sin bloqueo, cuarentena ni aviso por voz.
- [ ] **Adaptador del router** (DHCP, vecinos, Wi-Fi) según su firmware real. Sin eso o
  un puerto espejo, un nodo conmutado solo ve su propio tráfico.
- [ ] **Imprimir dentro del motor**: `Intent`, policy, journal y aprobación por digest.
  Hoy `archie-print` es una herramienta de operador que habla IPP directo.
- [ ] **Imprimir en una impresora real.** La Brother DCP-T720DW no respondía en la red.
- [ ] **Reglas sin prueba en vivo**: `dns-tunnel-suspect`, `address-moved`, `host-sweep`.
  Umbrales sin calibrar contra tráfico real.
- [ ] **Unidad `archie-netwatch.service`** sin probar bajo systemd.
- [ ] **Control más allá de Home Assistant**: Roku se lee y no se actúa; nada de Matter,
  MQTT, ONVIF ni Chromecast.
- [ ] Estado de la auditoría sin límites al cargar; deduplicación de alertas en memoria.

## 4. Contexto personal y trabajo

- [ ] **`PillarContext` gradúa todo `restricted`**, y `restricted` no sale a la nube: en
  un nodo sin modelo local esos pases terminan en rechazo. Hace falta un modelo local o
  que Cerebra gradúe por contenido y use `ask_private` (el velo).
- [ ] **Transporte Enterprise.** `POST /sync` responde 501: no hay envío a ningún tablero.
- [ ] Productores reales de Sentry, Wanda, Vault, IRIS y Forge hacia `PillarContext`.

## 5. Voz

Detalle: [`HANDOFF-2026-10-11-SPEECH-FLOOR.md`](HANDOFF-2026-10-11-SPEECH-FLOOR.md). — Sheriff

- [ ] Voz dentro de la imagen WSL (hoy no va).
- [ ] La voz y Arena clásico no pasan por el router ni por `ask_private`.
- [ ] Magpie y su envoltorio RVC en vivo; la detección de idioma es heurística.
- [ ] Reconocimiento del hablante. Es distinto de presencia y de autorización.

## 6. GUI profesional (Quickshell + Vault)

Detalle: [`gui/`](gui/README.md). El prototipo de `display/shell/prototype` es diseño:
Qt Quick puro con datos sintéticos, sin IPC ni acceso a archivos. — Xavi / Claude

- [ ] **Elegir la dirección**: la síntesis propuesta o una de las tres. Es de Xavi.
- [ ] **Aceptar o rechazar los ADR** de `gui/ARCHITECTURE_GUI.md`; siguen propuestos.
- [ ] **Conectar a datos reales** por el bridge de solo lectura: dominio (personal,
  trabajo), autoridad del Stage, presión de CPU. Hoy todo es `Synthetic.qml`.
- [ ] **10 de 15 superficies sin dibujar** (`gui/SCREEN_SPECS.md`).
- [ ] **Pausa de emergencia de autoridad**: `archied` no expone el control.
- [ ] **Vault**: 24 acciones `GUI-VAULT-*` derivadas de cosmic-files, sin implementar.
- [ ] Sin verificar: Quickshell, blur del compositor, NVIDIA, Wayland, lectores de
  pantalla, presupuesto de frame.
- [ ] **Orbe con el diseño nuevo de Lumenfield**: lo que hay en `display/native` es de
  septiembre; el diseño nuevo de Xavi no está subido.

## 7. Plataforma

- [ ] **Módulos**: 31 manifiestos siguen `unimplemented`.
- [ ] **Modelo local para datos `restricted`** (6–8 GB): manifiesto y servidor.
- [ ] **Unidad de servicio del Stage** en la ISO.
- [ ] **Un solo prompt de planner**: `ARCHIE.md` es el contrato; Arena aún lleva el suyo.
- [ ] **Prueba de punta a punta** con voz → planner → IRIS → resultado verificado → REM.
- [ ] Compatibilidad multiplataforma: [`TODO-COMPATIBILIDAD-MULTIPLATAFORMA.md`](TODO-COMPATIBILIDAD-MULTIPLATAFORMA.md).
- [ ] Biometría real, conectores de nodos y móviles, app iOS.
- [ ] Trello al día con esta lista. — Nelson
