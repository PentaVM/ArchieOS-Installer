# Bitácora de integración nativa — 15 octubre 2026

## Alcance y estado

Trabajo exclusivo en `PentaVM/ArchieOS-Nvidia`, PR41. Se partió de `eeeb68c` y se
incorporaron los nuevos commits de Xavi `1177407` y `17a8bee` mediante rebase de
nuestros commits y reconciliación de los cambios locales. Se conservan Wm, las luces,
drag, minimizar/restaurar, switcher, menú del dock y descarte de notificaciones.
No se ha fusionado el PR ni se ha reconstruido el release WSL que prueba Sheriff.

La GUI ahora es cliente nativo C++/Qt del backend existente. No se portó todo a
Rust, no se creó un Core de Windows ni se cambió la autoridad de Cortex, IRIS o
Policy. Los contratos nuevos son de lectura. La interpretación de «low level» es
un componente nativo integrado en la sesión y su presupuesto; Qt no se ejecuta en
el kernel. Arquitectura y matriz de capacidades: [NATIVE_INTEGRATION.md](NATIVE_INTEGRATION.md).

## Cambios

- `displayd` proyecta catálogo, proveedores minimizados, DTO de trayectorias de
  Cortex y workflows existentes. Compatible con campos ausentes y Core anterior;
  mantiene procedencia, límites y estados de indisponibilidad/frescura.
- `archie-display --shell` hospeda la shell y el viewport Lumenfield existentes.
  `ShellRuntime` consume un hijo `archie-shell-bridge watch` con argumentos fijos,
  límite de trama y reconexión; termina al cerrar el host. Sin WebEngine/Python.
- Un viewport posee el controller. Render oculto y presupuesto cero detienen
  timers/consultas; movimiento reducido no destruye la preferencia de giro. Se
  descartan respuestas de una lens anterior. El límite de entidades solicitado
  también se respeta en la composición Rust.
- Settings/Inspector/Forge/Actividad/Notificaciones consumen proyecciones reales.
  Vault operativo no muestra archivos/copia/undo inventados. Las vistas no
  simulan leases, bloqueo de sesión, selección de dominio ni aprobaciones de Policy.
- Se conservan cinemáticas, capas y Antes del Observatory. Presets nuevos y CLI
  aditivos; voz añade presets/paneles locales y espera ACK. World permanece en
  Observatory y se rechaza expresamente en la shell.
- Se mantienen los cambios de Xavi sobre ventanas y avisos, con dominios sin
  lectura. Descartar un aviso no altera el job; un nuevo estado vuelve a aparecer.
  Listas largas tienen scroll; ventanas iniciales caben en el escritorio pequeño.
- Claro beta mediante tokens compartidos y selector en Apariencia, después de
  las integraciones. Oscuro por defecto, preferencia local de sesión. El servicio
  y el build incluyen el bridge sin ampliar grupos/permisos del renderer.

## Evidencia

| Verificación | Resultado |
|---|---|
| Rust: CLI, displayd, shell-bridge, Orb, voz; `--locked` | 164 pruebas, 0 fallos |
| Clippy displayd/shell-bridge/CLI, todos sus targets, `--no-deps -D warnings` | salida 0 |
| rustfmt 1.88.0 sobre fuentes Rust modificadas | salida 0 |
| QML con QtTest, offscreen/software y estilo Basic | 75 casos, 0 fallos, stderr vacío |
| Suites anteriores BridgeModel / Surfaces / Wm | 21 / 88 / 44 comprobaciones, 0 fallos |
| CMake nativo y CTest | build correcto, 3 suites correctas |
| Bash `-n`: build y lanzadores | salida 0 |
| Host nativo → hijo Rust real → displayd fixture → QML | procedencia fixture confirmada |
| IPC visual | cinemática `crown` y panel Settings aceptados con `ok` |
| Render real Qt 6.11.2 / Windows / OpenGL | AMD Radeon integrada; ambos temas |

Se observaron primero capturas sin Orbe: el arnés arrancaba oculto y el gobernador
aplicaba correctamente intervalo cero. Con ventana expuesta se confirmó presupuesto
16 ms, ownership y controller activo. El modo fixture de snapshots no alimenta
TFOG: la esfera decorativa puede dibujarse pero no acredita entidades ni actividad
real. La ingestión de una escena con nodos se prueba separadamente en CTest.

Las capturas nuevas son `33-native-orb-fixture.png` y `34-native-light-beta.png`.
Son capturas del ejecutable real, no diseños generados. Se corrigió el recorte de
Ajustes observado a 1370×749, con regresión RED/GREEN. El arnés reproducible está
en `_dev/verification/xavi/run-native-shell.ps1`; sólo detiene procesos propios.

## Límites y pendientes antes de considerar un merge

- [ ] Ejecutar imagen instalada WSL/Linux con los productores reales de
  Cortex/IRIS/voz/Stage y verificar sus rutas/permisos de proyección.
- [ ] Validar NVIDIA/3060 Ti, DRM/KMS, WSLg, audio NeMo/live completo, otros Qt y
  plataformas. No se midieron FPS reales, VRAM ni latencia de inferencia.
- [ ] Benchmarks: arranque frío/caliente, presentación p50/p95/p99, CPU/RAM/VRAM,
  12/60/120/220/360/420 nodos, perfiles, minimizar/restaurar, presión y juego
  concurrente; registrar hardware/driver/backend exactos y fuentes del workload.
- [ ] Publicar las proyecciones de LAN/jobs legibles para la cuenta de display en
  la instalación, sin darle pertenencia al grupo de autoridad de Core.
- [ ] Contratos gráficos tipados para archivos/Vault, revisión de Policy,
  auditoría/alertas unificadas, envío/cancelación de trabajos y dominios reales.
  Las capacidades del backend existentes no se declaran ausentes por carecer de GUI.
- [ ] Persistencia/tema global, lectores de pantalla y pruebas de ventanas externas
  bajo el compositor real. Wm sólo administra vistas QML locales.
- [ ] Revalidar contra `main` actualizado antes de cualquier merge autorizado.

El límite nativo actual es 420 nodos: las opciones WebEngine 2.500/5.000 se
adaptaron a límites reales, sin prometer ese volumen. El Clippy ampliado a Orb
como target encontró siete avisos previos en `model.rs`/`lib.rs`; no se hizo una
refactorización ajena a esta integración. Los cuatro avisos de campos no usados
del binario voice sin sus features de audio también son previos. El build Qt
6.11 emite la advertencia de política CMake QTP0004; no impidió compilar/renderizar.

No presentar estos resultados como una prueba de actuadores, credenciales,
inferencias, un compositor completo ni una GUI ejecutándose en el kernel.

La revisión independiente final encontró una desincronización entre presets por
CLI/voz y Settings. Se corrigió sincronizando el estado compartido antes de aplicar
el perfil; la regresión RED/GREEN cubre el paso Neural → Esfera. La revisión confirmó
la corrección. La suite completa final quedó en 75 casos.

`cargo +1.88.0 fmt --all -- --check` no pasa en el workspace completo: incluye
fuentes previas sin formatear. Se reprodujo el fallo con `rustfmt 1.88.0` sobre
`archie-arena-live.rs` extraído del commit intacto `17a8bee`. No se reformatearon
crates ajenos para ocultarlo; las fuentes modificadas por esta integración sí
pasan con la versión fijada por CI. El resultado de CI remoto debe comprobarse
sobre la nueva cabeza, sin reutilizar los checks del commit anterior.
