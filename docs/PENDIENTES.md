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
- [x] `PentaVM/archie-iris#27` fusionado (float32 en GPU sin bf16 nativo, `einops`).

## 3. Sentry — Kimi K3 trabaja aquí

**Empieza por leer**, en este orden: [`HANDOFF-2026-10-11-SENTRY-AUDITORIA.md`](HANDOFF-2026-10-11-SENTRY-AUDITORIA.md)
(qué se probó, qué no, doce defectos conocidos), [`SENTRY-NETWATCH.md`](SENTRY-NETWATCH.md),
[`SENTRY-LAN.md`](SENTRY-LAN.md) y `core/crates/archie-sentry/README.md`.

**Dónde está el código:**

| Pieza | Ruta | Estado |
|---|---|---|
| Inventario activo (SSDP, mDNS, Roku) | `core/crates/archie-lan/src/{ssdp,mdns,upnp,roku,inventory}.rs` | probado en una red real |
| Captura y auditoría continua | `core/crates/archie-lan/src/{capture,audit}.rs`, `bin/archie-netwatch.rs` | 3 de 6 reglas probadas en vivo |
| Importador pasivo de TShark | `core/crates/archie-core/src/passive_network.rs` | solo mDNS IPv4 desde archivo |
| Motor: intents, policy, journal, recibos | `core/crates/archie-sentry/src/{contracts,engine,journal}.rs` | probado con PostgreSQL aislado |
| Adaptadores | `core/crates/archie-sentry/src/adapters/{home_assistant,ipp}.rs`, `ipp/print.rs` | HA con fixture; IPP sin impresora real |
| Operador y Core | `core/crates/archie-core/src/{sentry,sentry_config,operator,operator_session}.rs` | sesión de operador solo en Linux |

**Reglas que no se rompen:** C++ y Rust, nada de Python. Sentry observa y propone; actuar
pasa por `Intent` → policy → aprobación del operador → postcondición medida. Nada de
identificar personas por su teléfono ni mandar datos de la red a la nube. Una prueba con
fixture se reporta como fixture, no como dispositivo real. Un PR por avance, contra `main`.

**En este orden** (cada punto dice cuándo está hecho):

- [ ] **1. Una sola vista de dispositivos.** Unir `archie-lan` (inventario),
  `archie-netwatch` (`state.json`) y `passive_network` en un registro por dispositivo,
  con clave por dirección de hardware y la IP como atributo. *Hecho cuando:* un comando
  (`archie sentry-devices --json`) devuelve cada dispositivo una vez, con qué fuente dijo
  qué, y hay prueba con las tres fuentes describiendo el mismo equipo.
- [ ] **2. Llevar esa vista al world graph** por `archie-worldd`. *Hecho cuando:* el
  Observatory dibuja los dispositivos de la red real desde el grafo y no desde un archivo.
- [ ] **3. Alertas → `PillarContext` → Cerebra.** Hoy nada produce un `PillarContext`.
  Cada alerta de `archie-netwatch` entra como referencia del pilar Sentry. *Hecho
  cuando:* un escaneo de puertos generado a propósito termina en un pase de Cerebra con
  su traza, y la prueba lo recorre de punta a punta. Ver la sección 4: el pase se
  gradúa `restricted` y sin modelo local se rechaza.
- [ ] **4. Persistir lo que ya se avisó.** La deduplicación de alertas vive en memoria:
  tras reiniciar se repiten, y `address-moved` avisa una vez por par y luego calla aunque
  dos equipos peleen por la IP del router. *Hecho cuando:* reiniciar no repite alertas y
  un vaivén de ARP produce una alerta que cuenta los cambios.
- [ ] **5. Límites al cargar el estado.** `state.json` se deserializa sin topes. *Hecho
  cuando:* un archivo con más dispositivos, nombres o destinos que los máximos se rechaza
  con error, con prueba.
- [ ] **6. Probar en vivo las tres reglas que faltan**: `dns-tunnel-suspect`,
  `address-moved`, `host-sweep`. En WSL el DNS sale por loopback; hace falta Linux real o
  capturar en `any`. *Hecho cuando:* cada una salta con tráfico generado y queda el
  comando que lo reproduce.
- [ ] **7. Baselines por dispositivo**: volumen y destinos habituales, alerta por
  desviación. Los umbrales actuales (100 puertos, 50 hosts, entropía 4,0) son criterio,
  no medida. *Hecho cuando:* hay una tasa de falsos positivos medida sobre una captura
  real de al menos un día.
- [ ] **8. Adaptador del router** (DHCP, vecinos, Wi-Fi), según el firmware real: el de
  Nelson es un MitraStar GPT-2741GNAC. Sin eso o un puerto espejo, un nodo conmutado
  solo ve su propio tráfico. *Hecho cuando:* la lista de clientes del router entra a la
  vista del punto 1, solo lectura.
- [ ] **9. Imprimir dentro del motor.** `archie-print` habla IPP directo y se confirma
  escribiendo `print`. Debe ser un `Command::Print` con policy, journal y aprobación por
  digest. El motor exige plazo ≤ 30 s y un trabajo tarda más: proponer la postcondición
  (por ejemplo «la impresora tiene el trabajo N») antes de implementar. *Hecho cuando:*
  `archie sentry-execute <impresora> print …` deja un recibo durable.
- [ ] **10. Imprimir en una impresora real.** La Brother DCP-T720DW no respondía. *Hecho
  cuando:* sale una página y el recibo dice `completed`, con el formato que aceptó.
- [ ] **11. Respuesta.** Hoy todo es observación. Primera respuesta reversible: aviso
  por voz y por el Observatory. Bloqueo o cuarentena solo como `Intent` aprobado.
- [ ] **12. Control más allá de Home Assistant**: Roku ECP se lee y no se actúa; nada de
  Matter, MQTT, ONVIF ni Chromecast. Cada uno es un adaptador con su prueba.
- [ ] **13. `archie-netwatch.service` bajo systemd real.** No se sabe si `DynamicUser`
  con capacidades ambientales basta para `dumpcap` en Arch.
- [ ] 14. IPv6: `address-moved` solo mira ARP; falta Neighbor Discovery.

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

## 6. Diseño — Xavi termina aquí

**Lo que hay** (commit `4b282cd`, ya en `main`): `display/shell/prototype/`, doce
componentes QML con `Theme.qml` (tokens de las tres direcciones y la síntesis) y
`Synthetic.qml` (todos los datos, inventados y rotulados). Documentos en
[`gui/`](gui/README.md): `DESIGN_SYSTEM.md`, `SCREEN_SPECS.md`, `MOTION_CINEMATICS.md`,
`ARCHITECTURE_GUI.md`, `GUI_FUNCTION_BACKLOG.md`. Capturas en `gui/screenshots/`.

**Lo que no hay:** nada del diseño nuevo de **Lumenfield para el Orbe**. El Orbe de
`display/native` (`LumenScene.qml`, `LumenField.cpp`, shaders `lumen_orb.*`) es el de
septiembre, y desde `4b282cd` no entró ningún commit de diseño. Si existe fuera del
repositorio, hay que subirlo.

**Decisiones que son de Xavi** (bloquean lo demás):

- [ ] **D1. La dirección visual.** Frost, Workspace, Signature o la síntesis propuesta
  (superficies de Frost, acento `#6fc8f5`, radio 12, densidad 0,94). Dejarla como la única
  en `Theme.qml`.
- [ ] **D2. Tipografía.** El prototipo usa la del sistema; las capturas salieron con
  Segoe UI porque se renderizaron en Windows.
- [ ] **D3. Los ADR** de `gui/ARCHITECTURE_GUI.md`: aceptar o rechazar cada uno.
- [ ] **D4. Lumenfield nuevo para el Orbe.** Subir el diseño y decir qué reemplaza de
  `display/native`. El acento de la síntesis se eligió para convivir con el cian del
  Orbe actual (`#6fe3f5`); si el Orbe cambia, revisar ese par.

**Diseño por terminar:**

- [ ] **10 de 15 superficies sin dibujar** (lista en `gui/SCREEN_SPECS.md`). Dibujadas:
  barra superior, dock, Vault (lista y cuadrícula), centro de control, centro de
  actividad, paleta de comandos.
- [ ] **Movimiento**: `MOTION_CINEMATICS.md` es especificación; nada está animado.
- [ ] **Estados** de cada superficie: vacío, cargando, error, sin permiso, sin conexión.
- [ ] **Vista de Sentry**: dispositivos de la red y alertas (depende del punto 1 de
  Sentry). Hoy no tiene superficie.
- [ ] **Vista del Stage**: quién tiene la autoridad y cómo se toma el control.

**Integración** (Claude, cuando D1–D3 estén decididas):

- [ ] **Conectar a datos reales** por el bridge de solo lectura (`display/native/src/ArchieBridge.h`),
  reemplazando `Synthetic.qml` superficie por superficie. Primero: dominio
  (personal o trabajo), autoridad del Stage, presión de CPU.
- [ ] **Chips de contexto de Vault** desde `ContextDomain` (`core/crates/archie-vault/src/domain.rs`).
- [ ] **Pausa de emergencia de autoridad**: `archied` no expone ese control; el
  prototipo ya lo muestra como no disponible.
- [ ] **Vault**: las 24 acciones `GUI-VAULT-*` de `gui/GUI_FUNCTION_BACKLOG.md`.
- [ ] **Sin verificar**: Quickshell, blur del compositor, NVIDIA, Wayland, lectores de
  pantalla, presupuesto de frame. El prototipo solo se renderizó por software.

## 7. Plataforma

- [ ] **Módulos**: 31 manifiestos siguen `unimplemented`.
- [ ] **Modelo local para datos `restricted`** (6–8 GB): manifiesto y servidor.
- [ ] **Unidad de servicio del Stage** en la ISO.
- [ ] **Un solo prompt de planner**: `ARCHIE.md` es el contrato; Arena aún lleva el suyo.
- [ ] **Prueba de punta a punta** con voz → planner → IRIS → resultado verificado → REM.
- [ ] Compatibilidad multiplataforma: [`TODO-COMPATIBILIDAD-MULTIPLATAFORMA.md`](TODO-COMPATIBILIDAD-MULTIPLATAFORMA.md).
- [ ] Biometría real, conectores de nodos y móviles, app iOS.
- [ ] Trello al día con esta lista. — Nelson
