# Integración nativa de la shell

La shell se hospeda en `archie-display --shell`, el mismo ejecutable C++/Qt Quick 3D
del Observatory. Reutiliza Lumenfield, shaders, picking y controles cinematográficos.
No requiere Chromium, WebEngine ni el servidor Python experimental.

Core conserva modelos, adaptadores, planificación, Policy y leases. El display sigue
siendo un cliente sin autoridad: `archied → displayd → shell-bridge → QML`. Las nuevas
rutas GET `/api/inspector` y `/api/workflows` proyectan contratos existentes y eventos
observados. No son un motor de razonamiento adicional. El presupuesto del gobernador
limita intervalos, partículas y efectos; minimizar/ocultar detiene el render activo.
QML/C++ siguen en espacio de usuario; no se introdujo código gráfico en el kernel.

## Ejecutar

Construir `archie-displayd`, `archie-shell-bridge`, `archie-cli` y el target CMake de
`display/native`. Con los binarios instalados:

```sh
archie-display --shell --shell-bridge /usr/bin/archie-shell-bridge
archie-display --shell --theme light     # claro beta; oscuro por defecto
archie visual list
archie visual preset neural
```

`display/shell/start-native.sh` es el lanzador para una sesión existente (incluido
WSLg). No inicia otro Core, no elige una GPU fija ni modifica servicios. El servicio
DRM/KMS `archie-display.service` conserva su cuenta aislada, sandbox y límites.
Para recuperación, `archie-display --scene world` mantiene el Observatory original.
World no está embebido en la shell: se rechaza explícitamente ese cambio por IPC.

El puerto local 7668 acepta sólo vistas/presets/cinemáticas/capas permitidas. La voz
conserva `observatory_scene`, `orb_cinematic`, `orb_overlay` y añade `orb_preset` y
`shell_panel`. Espera `ok` del display antes de informar un cambio visual. Ninguna
de estas herramientas puede aprobar una operación de Core.

## Capacidades y límites

| Área | Conectado | Pendiente real |
|---|---|---|
| Cortex | DTO de trayectorias minimizadas | Contenido privado no se expone |
| IRIS/voz/Stage | Eventos de workflow observados, con edad y procedencia | La ausencia de señal no acredita servicio detenido |
| Modelos/proveedores | Catálogo real y disponibilidad declarada | Carga/cambio de modelos sigue bajo Core |
| Forge | Jobs existentes y manifiestos | Enviar/cancelar desde la GUI necesita contrato tipado |
| Vault | Vista local preparada, sin operaciones ficticias | Listado/operaciones reales y separación de dominios con autoridad |
| Policy | Estado de indisponibilidad explícito | Canal gráfico de revisión/aprobación y auditoría |
| Sentry | Inventario que produce el colector | Permisos de lectura de la proyección en cada instalación |
| Ventanas | Gestión local de vistas QML de Xavi: foco, mover, minimizar, restaurar | No controla ventanas de otras aplicaciones ni sustituye un compositor |
| Notificaciones | Jobs reales; descarte/restauración local | Historial unificado de Policy/Sentry |
| Apariencia | Oscuro y claro beta, movimiento reducido, opacidad | Preferencias de sesión; integración con preferencia global/persistencia |

El renderer nativo limita actualmente la vista a **420 nodos**, además del presupuesto
del OS. Las opciones del visor WebEngine de 2.500/5.000 no se presentan como soportadas
por este renderer. Falta medir y ampliar ese límite con perfiles de rendimiento.
Los presets nuevos son aditivos: se conservan `field/crown/atlas/dense`, las capas y
la comparación Antes del Observatory. Se añade `sphere/orb/neural`.

Las grabaciones se fuerzan a `fixture`; silencio/lecturas ausentes no son éxito ni
inventario vacío. LAN y jobs mantienen sus fuentes independientes del estado de Core.
El usuario `archie-display` no gana pertenencia al grupo de autoridad de Core para
leer archivos privados: deben publicarse proyecciones adecuadas en la instalación.

## Verificación reproducible

```sh
cd core
cargo test -p archie-displayd -p archie-shell-bridge -p archie-orb --locked
cargo test -p archie-cli -p archie-voice --locked
cd ..
cmake -S display/native -B build/native -DARCHIE_NATIVE_BUILD_TESTS=ON
cmake --build build/native
ctest --test-dir build/native --output-on-failure
QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software QT_QUICK_CONTROLS_STYLE=Basic \
  QML_XHR_ALLOW_FILE_READ=1 qmltestrunner -input display/shell/prototype
```

La prueba visual real requiere una API RHI/OpenGL/D3D compatible, **sin**
`QT_QUICK_BACKEND=software`; el backend software de Qt Quick no dibuja Quick3D.
Para probar transporte sin modelos ni dispositivos:

```sh
archie-displayd --fixture _dev/verification/xavi/native-shell-fixture.json --disable-world-feed
archie-display --shell --shell-bridge /ruta/archie-shell-bridge \
  --render-check /ruta/nueva/dark.png
archie-display --shell --theme light --shell-bridge /ruta/archie-shell-bridge \
  --render-check /ruta/nueva/light.png
```

Sólo usar un puerto de prueba libre; no detener servicios ajenos. El JSON de ejemplo
contiene datos inventados explícitos. Una captura fixture no verifica inferencia,
actuadores, ASR/TTS ni aceleración NVIDIA. No reemplaza el test de Sheriff en su WSL.
El modo fixture de snapshots no alimenta entidades TFOG: el Orbe vacío es correcto.
Se valida la ingestión de una escena con nodos en `native_contracts`; para validar
el dibujo completo con datos vivos hace falta el Core y sus productores reales.
