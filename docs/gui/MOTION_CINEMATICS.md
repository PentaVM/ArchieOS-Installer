# Cinemáticas y movimiento

Estado: **diseño; casi nada implementado.** En el prototipo sólo existe el escalado del dock al pasar el ratón (120 ms, `OutCubic`) y los tokens de duración con `reducedMotion`. Cada fila siguiente es una **especificación**, no una animación existente.

Regla del handoff: una cinemática explica causalidad y estado; no sustituye al feedback operativo ni se dispara por temporizador.

## Perfiles

| Perfil | Efectos | Disparo |
|---|---|---|
| **Full** | todos los movimientos, vidrio si el compositor lo ofrece | GPU libre |
| **Balanced** | sin blur, mismos movimientos | `display_budget` = `balanced` |
| **Quiet / AI Workload** | sólo transiciones de opacidad ≤ 120 ms, sin vidrio | `display_budget` reducido o IRIS/LLM activos (`archie-displayd` `GET /api/kernel`, ya existente) |
| **Reduced Motion** | cero movimiento espacial; cambios de estado instantáneos y con el glifo/palabra intactos | preferencia del usuario (`Theme.reducedMotion`, tecla `M` en el prototipo) |

Toda cinemática lleva **bandera de capacidad** y **fallback** a "cambio de estado sin animación".

## Catálogo (especificación)

| # | Cinemática | Qué cuenta | Disparo real exigido | Presupuesto propuesto | Fallback | Estado |
|---|---|---|---|---|---|---|
| 1 | Abrir app desde dock/launcher | de dónde viene la ventana | evento `launched` / `surface` real (`AppCatalog::launched`, `StageEvent::Surface`) | 180 ms, transform+opacity | aparece directa | no implementada |
| 2 | Cerrar/minimizar/restaurar | adónde va | evento `Unmapped` real | 140 ms | desaparece | no implementada |
| 3 | Overview / cambio de escritorio | continuidad espacial | cambio de workspace del compositor | 220 ms | corte | no implementada |
| 4 | Paleta de comandos | respuesta al teclado | tecla | 160 ms elástica suave; resultados incrementales sin animar el reflow | aparece | estática en prototipo |
| 5 | Vault: entrar a carpeta, lista↔iconos, selección | jerarquía | navegación real | 120 ms crossfade | corte | estática |
| 6 | Vault: arrastrar/soltar y progreso de copia | causalidad de la operación | **progreso real** de `archie-vaultd` (no existe) | barra ligada a eventos | barra sin animar | BLOCKED |
| 7 | Vault: previsualización progresiva y miniaturas perezosas | que algo carga | carga real | esqueleto sin bloquear UI | icono genérico | no implementada |
| 8 | Notificaciones | aparece, se agrupa, se retira sin saltos | notificación real | 140 ms | aparece | no implementada |
| 9 | Centro de control | panel flotante, respuesta del servicio | respuesta real de cada servicio | 160 ms | aparece | estática |
| 10 | Diálogo de autorización | seriedad, cancelable | escalada real de Policy (**sin IPC hoy**) | 160 ms, sin rebote | aparece | BLOCKED |
| 11 | Sentry: conexión/desconexión de dispositivo | cambio de estado | transición real de estado | pulso único (1×) | sin pulso | no implementada; **jamás dibujar éxito falso** |
| 12 | Forge: etapas de pipeline | progreso | evento de etapa real | chip por etapa | chip estático | BLOCKED (módulo sin worker) |
| 13 | Feed de Watcher/observaciones | llegada de eventos | observación real | 120 ms, agrupado | aparece | no implementada |
| 14 | Orbe ↔ Workspace | continuidad sin destruir | **cambio de workspace del compositor** (patrón de `archie-session.sh`) | 280 ms crossfade a nivel de compositor | corte | no implementada; **el render del Orbe no se toca** |
| 15 | Offline / error / reconexión / degradado | la verdad | latido de `archied` (`ping`) | chip con edad del dato; sin efecto decorativo | idem | chip en prototipo |
| 16 | Arranque / sesión / desbloqueo | tras autenticación real | auth real | n/d | n/d | no implementada |

## Implementación preferida

Animaciones de Qt Quick (`Behavior`, `NumberAnimation`, `Transition`) sobre `transform` y `opacity`. Sombras sin shader. Shaders sólo donde un efecto lo justifique y con variante precompilada; nunca blur ni shader a pantalla completa en perfiles Balanced/Quiet. Sin temporizadores de CPU agresivos; sin recrear árboles QML enteros.

## Medición pendiente (no hay números)

p50/p95/p99 de tiempo de fotograma, jank, RAM/VRAM, latencia entrada→pantalla aproximada y consumo en reposo, en 1080p/1440p y con IRIS ocupando la GPU (RTX 3070 8 GB). Objetivo a probar, no resultado: 60 FPS estables en 1080p, 120 Hz si compositor y hardware lo permiten. Los presupuestos de la tabla son propuestos hasta tener línea base.
