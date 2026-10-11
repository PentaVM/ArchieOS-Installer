# TODO maestro: compatibilidad amplia de ArchieOS

Fecha: 2026-10-11.
Estado: backlog solicitado por el operador. La implementación y publicación autorizadas se registran en la bitácora; esta lista no certifica capacidades ni modifica los permisos del producto.
Todas las casillas empiezan pendientes: incluso una capacidad existente necesita evidencia de compatibilidad por plataforma. Pendiente no significa ausente.

> [!IMPORTANT]
> **LA COMPATIBILIDAD AMPLIA ES UN REQUISITO DE PRODUCTO.**
> ArchieOS debe aprovechar el hardware que el operador ya tiene y adaptarse al que venga. Ninguna decisión debe atarlo a una sola GPU, arquitectura de CPU, marca o sistema anfitrión.
> La aspiración es que funcione en la mayor diversidad posible: Mac mini, DGX Spark, RTX Spark, AMD Strix Halo, servidores antiguos, GPUs de consumo actuales y antiguas, RTX 2080 y distintas generaciones de NVIDIA Titan.
> Esta aspiración exige pruebas, backends reemplazables y degradaciones útiles. No permite anunciar «compatible con todo» sin evidencia, ni rebajar permisos para conseguir que algo arranque.

> [!WARNING]
> **WINDOWS DEBE SEGUIR SIENDO UN PC DE USO GENERAL.**
> Los juegos mencionados antes eran ejemplos. El requisito incluye la biblioteca Steam del operador, aplicaciones habituales, periféricos y trabajo diario, mientras ArchieOS opera en paralelo.
> Archie debe poder controlar el host cuando tenga autorización: archivos, aplicaciones, entregas, ventanas y operaciones concretas. El aislamiento no debe eliminar esas funciones; debe delimitar y auditar su acceso.
> «Sin regresiones de compatibilidad» y «sin impacto perceptible» son objetivos que deben medirse por escenario. No equivalen a garantizar cero contención ni compatibilidad de todos los juegos existentes.

## 1. Invariantes que ninguna portabilidad puede romper — P0

- [ ] Preservar la autoridad del operador y los contratos Core/Policy/leases.
- [ ] Preservar Cortex/Cerebra y su separación respecto de la autoridad determinista.
- [ ] Preservar IRIS, Reflex, especialistas, Scene Graph, ACP y TTL donde estén implementados; auditar antes de reemplazar.
- [ ] No crear un segundo Cortex, scheduler ni policy engine en el host.
- [ ] Mantener inferencia y outputs de modelos como trabajadores subordinados.
- [ ] Mantener proveedores intercambiables sin bifurcar el comportamiento central.
- [ ] Mantener el camino de ArchieOS instalado nativamente e imagen de VM.
- [ ] No convertir WSL, Docker, Windows, CUDA ni un proveedor comercial en identidad obligatoria del producto.
- [ ] Rechazar una operación si el backend no aplica el enforcement requerido.
- [ ] Separar autoridad lógica, aislamiento entre procesos y protección frente al administrador del host.
- [ ] Preservar el funcionamiento local sin Internet cuando los artefactos necesarios estén provisionados.
- [ ] Establecer gates de regresión para permisos, routing, ejecución, recuperación y memoria antes de integrar backends.

## 2. Inventario y reconciliación del código existente — P0

- [ ] Inventariar repositorios, ramas, worktrees y revisiones exactas antes de dimensionar trabajo.
- [ ] Comparar `Archie` con `ArchieOS-nvidia`, `ArchieOS-nvidia-bridges` y sus integraciones.
- [ ] Localizar la revisión canónica acordada; no inferirla del nombre de la carpeta o de la fecha.
- [ ] Auditar el IRIS Rust encontrado en otros checkouts y el IRIS Python/PyTorch separado.
- [ ] Distinguir funcionalidades compiladas, empaquetadas, configuradas, ejecutadas y probadas con hardware.
- [ ] Reconciliar documentación histórica contradictoria sin borrar su evidencia.
- [ ] Inventariar interfaces de modelos, transporte, eventos, almacenamiento y control de host.
- [ ] Inventariar dependencias nativas, SIMD, ABI, formatos y binarios por arquitectura.
- [ ] Documentar capacidades que ya existen para no recrearlas.
- [ ] Registrar diferencias locales no comprometidas sin sobrescribirlas.
- [ ] Auditar `archie-desk`, `archie_deliver`, outbox y reglas de apertura existentes.
- [ ] Verificar dónde está instalado el receptor de entregas y qué equipo es realmente el destino.
- [ ] Comprobar la integración runtime de contratos host; un enum no demuestra ejecución operacional.
- [ ] Crear matriz capacidad × plataforma × evidencia × limitación × responsable.

## 3. Matriz de hardware objetivo — P0/P1

Los siguientes son objetivos de investigación y validación, no declaraciones de soporte. «Halo» se interpreta como AMD Strix Halo; confirmar SKU exacto al disponer del equipo.

| Familia | Variantes que hay que distinguir | Prueba pendiente principal |
|---|---|---|
| Mac mini Apple Silicon | Generación, RAM, macOS, GPU integrada | Backend Metal, memoria, ARM64 y control del host |
| Mac mini Intel | CPU, iGPU y configuración real | CPU/iGPU disponible, x86_64 y límites del SO |
| DGX Spark | SKU, RAM, DGX OS, ARM64 y driver | Runtime nativo Linux y dependencia de paquetes |
| RTX Spark | SKU, portátil/desktop, TDP, RAM, Windows ARM | Windows ARM64, WSL ARM64, CUDA y apps/juegos |
| AMD Strix Halo | SKU, firmware, UMA, Windows/Linux | Backends AMD disponibles, presupuesto UMA y driver |
| NVIDIA RTX 2080 | SKU, VRAM, driver, SO | Kernels y formatos admitidos; convivencia |
| NVIDIA Titan | GTX TITAN/Black/Z, X Maxwell, X Pascal, Xp, V, RTX | Arquitectura, VRAM, driver y soporte exactos |
| NVIDIA servidor antiguo | K/P/V/T y modelos concretos | Driver legado, toolkit, ISA y ausencia de display |
| NVIDIA consumo antiguo | GTX y RTX por arquitectura | Ruta CUDA/Vulkan/CPU viable por versión |
| AMD discreta | Radeon consumo y Radeon Pro/Instinct concretas | ROCm/HIP/Vulkan por producto y SO |
| Intel | iGPU, Arc y dispositivos concretos | SYCL/OpenVINO/Vulkan según runtime |
| CPU sin GPU | x86_64 y ARM64, SIMD y RAM limitada | Inferencia reducida, voz y operación degradada |
| Varias GPU/nodos | Homogéneos y heterogéneos | Placement, memoria, transporte y recuperación |

- [ ] Registrar identificador PCI, arquitectura y VRAM de cada GPU de prueba.
- [ ] Separar memoria instalada, visible, utilizable por backend y disponible bajo carga.
- [ ] Registrar firmware, BIOS, driver, SO, kernel y toolkit.
- [ ] Distinguir formatos que el hardware acelera de formatos que el software puede emular.
- [ ] Verificar soporte del toolkit por generación; no instalar «latest» indiscriminadamente.
- [ ] Distinguir driver de display, TCC/WDDM y dispositivo sin salida de vídeo donde corresponda.
- [ ] Verificar si el servidor depende de GPU para boot o puede operar headless.
- [ ] Incluir memoria ECC, fallos de dispositivo y GPUs usadas con salud incierta.
- [ ] Tratar cada Titan como producto distinto; no usar «Titan compatible» como resultado global.
- [ ] Crear ficha específica para Titan como familia prioritaria solicitada por el operador.
- [ ] Definir mínimos de CPU, RAM, disco y aceleración por perfil, sin un único mínimo universal.
- [ ] Incorporar perfiles para equipos pequeños y notebook de administración sin obligarla a inferir.

## 4. Sistemas operativos, CPU y distribución — P1

- [ ] Validar Core Rust en x86_64 y ARM64.
- [ ] Validar Cortex y componentes C++ en ambas arquitecturas.
- [ ] Auditar SIMD, alineamiento, atomics y supuestos de orden de memoria.
- [ ] Validar modelos y plugins sin ejecutar binarios de arquitectura equivocada.
- [ ] Diseñar artefactos diferenciados para ISO x86_64 y distribución ARM64 cuando sea viable.
- [ ] Validar ArchieOS Linux nativo en hardware consumidor.
- [ ] Validar perfil Linux headless de servidor.
- [ ] Validar WSL2 x86_64 y systemd requerido.
- [ ] Validar WSL2 ARM64 por dispositivo real; no extrapolar del soporte x86_64.
- [ ] Validar Windows nativo x64 y ARM64 para agentes subordinados.
- [ ] Validar macOS Apple Silicon para runtime/worker y permisos de integración.
- [ ] Investigar Linux nativo en Apple Silicon por modelo; no prometer equivalencia con macOS.
- [ ] Validar Hyper-V, QEMU/KVM y VM de macOS donde proceda.
- [ ] Auditar Landlock, cgroups, systemd y namespaces por entorno.
- [ ] Especificar equivalentes Windows/macOS y límites donde no exista equivalencia.
- [ ] Documentar qué funciones del boot, kernel y display no se trasladan a WSL.
- [ ] Probar instalaciones offline con paquetes fijados.
- [ ] Probar actualización, rollback y recuperación por plataforma.
- [ ] Evitar que la emulación x86 se convierta en supuesto de rendimiento o de compatibilidad de drivers.

## 5. Proveedores de inferencia y selección de capacidades — P1

- [ ] Auditar el provider contract existente antes de proponer otro.
- [ ] Evaluar CUDA Linux nativo.
- [ ] Evaluar CUDA dentro de WSL2.
- [ ] Evaluar worker CUDA Windows x64/ARM64.
- [ ] Evaluar Metal para macOS.
- [ ] Evaluar ROCm/HIP por GPU y SO realmente soportados.
- [ ] Evaluar Vulkan para hardware fuera de los caminos anteriores.
- [ ] Evaluar SYCL/OpenVINO para Intel cuando aporten una ruta útil.
- [ ] Evaluar CPU SIMD como fallback operativo, no como sustituto silencioso de GPU.
- [ ] Comparar llama.cpp/GGUF y motores adicionales según modelo y plataforma.
- [ ] Mantener Python donde una dependencia existente lo requiera; no reescribir IRIS sin justificación.
- [ ] Priorizar Rust/C++ para supervisión y transporte críticos.
- [ ] Separar LLM, percepción, ASR, TTS y embeddings como capacidades de proveedores.
- [ ] Detectar aceleración mediante pruebas pequeñas; un nombre de GPU no basta.
- [ ] Publicar incompatibilidad con código de razón y alternativa disponible.
- [ ] Evitar que un backend más rápido eluda privacidad, leases o calidad mínima.
- [ ] Registrar compatibilidad de model format, tokenizer, plantilla y modalidades.
- [ ] Verificar GGUF/modelos híbridos y estados recurrentes por motor.
- [ ] Verificar LoRA por base exacta, arquitectura, revisión, módulos y formatos.
- [ ] Probar hot-swap, sesiones aisladas e invalidación de cachés.
- [ ] Probar speculative decoding solo con draft/target compatibles y calidad validada.
- [ ] Probar batching, flash attention y cuantización KV donde existan.
- [ ] Distinguir FP4 nominal, NVFP4, INT4 y kernels implementados.
- [ ] Ofrecer selección explícita y fallback con visibilidad del cambio.

## 6. Virtualización, IPC y memoria — P1

- [ ] Comparar guest CUDA con motor nativo host bajo el mismo workload.
- [ ] Mantener KV y estados de inferencia junto al motor siempre que el contrato lo permita.
- [ ] Evitar transferencia por token de pesos, KV o tensores innecesarios.
- [ ] Evaluar TCP autenticado/streaming como baseline portable.
- [ ] Evaluar gRPC sin atribuirle penalización por nombre.
- [ ] Validar Hyper-V sockets/vsock por host, guest y kernel.
- [ ] Validar AF_UNIX dentro de cada SO y no asumir interoperabilidad host/guest.
- [ ] Validar named pipes para procesos Windows.
- [ ] Investigar memoria compartida solo si una medición justifica su complejidad.
- [ ] Definir cuándo mmap/pinning/zero-copy realmente evitan una copia.
- [ ] Medir filesystem local frente a rutas compartidas y copias verificadas.
- [ ] No sumar RAM del host y guest como capacidad adicional.
- [ ] Medir presión de memoria UMA con GPU, CPU y juego simultáneos.
- [ ] Probar backpressure, límites de frames y cancelación en todos los transportes.
- [ ] Autenticar extremos y fijar identidades; loopback no equivale a autorización.
- [ ] Medir IPC de audio/vídeo separado del streaming de tokens.
- [ ] Evaluar passthrough solo con compatibilidad oficial o clasificación experimental explícita.
- [ ] No asumir particiones físicas ni QoS GPU garantizadas en hardware que no las ofrece.
- [ ] Mantener ggml RPC experimental fuera de producción sensible hasta revisión propia.
- [ ] Exigir caso medido y necesidad concreta antes de encargar drivers de kernel.

## 7. Windows de uso general, Steam y aplicaciones — P0/P1

- [ ] Registrar la biblioteca real Steam del operador como conjunto de aceptación.
- [ ] Registrar otras tiendas, launchers, aplicaciones y periféricos habituales.
- [ ] Probar juegos representativos DX11, DX12, Vulkan y OpenGL.
- [ ] Separar compatibilidad de arranque, online/anticheat y rendimiento.
- [ ] Validar anticheat, DRM y drivers ARM64 por título en plataformas ARM.
- [ ] No presentar frames generados como equivalentes a FPS base ni menor latencia.
- [ ] Probar fullscreen, borderless, cambio de ventana y varios monitores.
- [ ] Probar HDR, VRR, resoluciones, refrescos y reconexión de pantallas.
- [ ] Probar mando, teclado, mouse, audio y headset con Archie activo.
- [ ] Medir juego solo frente a juego + Archie con los mismos presets.
- [ ] Medir FPS, 1% lows, frametimes, stutter e input con herramientas apropiadas.
- [ ] Validar trabajo Office, CAD, edición, navegador, videollamadas y herramientas contables.
- [ ] Validar suspend/resume, hibernación y sesiones bloqueadas.
- [ ] Validar actualizaciones de Windows/driver que cambien WSL o GPU.
- [ ] Diseñar perfil de convivencia ajustable por el operador.
- [ ] Evitar robo de foco, teclas globales o cambios de audio inesperados.
- [ ] Permitir pausar trabajo pesado conservando eventos esenciales.
- [ ] Reportar títulos o aplicaciones incompatibles sin ocultarlos tras un resultado agregado.

## 8. Control autorizado del host y entrega de proyectos — P0/P1

- [ ] Conservar `archie-desk` y `archie_deliver`; validar antes de crear sustitutos.
- [ ] Probar recepción autenticada, hash, timestamp y replay en Windows/macOS/Linux.
- [ ] Probar outbox, reconexión, duplicados y entrega por etapas.
- [ ] Preservar originales y vincular resultados a proyecto, tarea y receipt.
- [ ] Validar catálogo de apertura autorizado por extensión/aplicación.
- [ ] Distinguir «proceso iniciado» de «archivo abierto correctamente» mediante receipts.
- [ ] Verificar que los tipos de archivo autorizados no introduzcan ejecución arbitraria no prevista.
- [ ] Auditar rutas, enlaces, reparse points, traversal y archivos incompletos.
- [ ] Probar entrega de XLSX/DOCX/PDF/CAD/media en aplicaciones reales.
- [ ] Añadir o validar selección de monitor principal/secundario donde exista API autorizada.
- [ ] Mantener entrega sin apertura automática durante juegos o reuniones según preferencia.
- [ ] Conservar opción «listo para revisar» y apertura solicitada por el operador.
- [ ] Reconciliar contratos `HostOperation` con implementaciones reales de import/export.
- [ ] Validar notificaciones, app launch, micrófono y portapapeles con permisos acotados.
- [ ] Investigar UI Automation/accesibilidad del host antes de input sintético genérico.
- [ ] Mantener captura y control de superficies atados a identidad, sesión, propósito y TTL.
- [ ] Separar compute worker de agente de control del escritorio; ninguno recibe autoridad general.
- [ ] Integrar prepare → policy/approval → lease → dispatch → receipt sin doble scheduler.
- [ ] Permitir revocación inmediata y pausa de emergencia de operaciones host.
- [ ] Respetar UAC, TCC/macOS y prompts del SO; no convertir incompatibilidad en bypass.
- [ ] Impedir controles sobre sesión/ventana distinta de la autorizada.
- [ ] Probar recuperación sin repetir acciones de resultado incierto.

## 9. Casa, voz, percepción y contabilidad — P1/P2

- [ ] Inventariar cámaras, codecs, sensores y protocolos realmente usados.
- [ ] Validar captura consentida y privacidad con Sentry/IRIS existentes.
- [ ] Medir varios streams y decodificación hardware por plataforma.
- [ ] Usar detección/eventos para activar análisis pesado cuando sea útil.
- [ ] Preservar TTL, Scene Graph, ACP y aislamiento de propuestas de acción.
- [ ] Probar ASR/TTS, barge-in y latencia bajo juego/inferencia simultáneos.
- [ ] Probar dispositivos de audio dedicados y cambios de dispositivo.
- [ ] Mantener tareas esenciales observables cuando el motor grande se pause.
- [ ] Definir qué monitorización se degrada en cada perfil y notificarlo.
- [ ] Conservar trabajo contable determinista para sumas, conciliaciones y controles.
- [ ] Validar extracción/clasificación con datasets y formatos del operador.
- [ ] Preparar entregables, excepciones y evidencias para revisión antes de envío.
- [ ] No confundir rendimiento del modelo con integridad o corrección del trabajo.
- [ ] Probar interrupción, checkpoint y reanudación de trabajos largos.

## 10. Benchmarks y aceptación reproducible — P0/P1

- [ ] Crear manifiesto por ejecución: hardware, software, hashes y configuración.
- [ ] Separar cold boot, carga de pesos, warm TTFT y primer resultado útil.
- [ ] Medir prefill, decode y latencia entre tokens p50/p95/p99.
- [ ] Medir contextos cortos, largos, nuevos y reutilizados.
- [ ] Medir LoRA, modelos híbridos y visión por separado.
- [ ] Medir carga pequeña, 27B y modelos adicionales realmente disponibles.
- [ ] Medir concurrencia y profundidad de cola.
- [ ] Medir RAM, VRAM/UMA, paging, disco, ancho de banda y memoria pinned.
- [ ] Medir energía, temperatura, throttling y rendimiento sostenido.
- [ ] Comparar Windows nativo, guest CUDA y bridge manteniendo el mismo motor.
- [ ] Comparar macOS Metal y Linux/Windows sin fingir que son controles equivalentes cuando difieren.
- [ ] Separar prueba del motor de prueba operacional completa de ArchieOS.
- [ ] Fijar warmups, repeticiones, semillas y longitud/tokenización de entradas.
- [ ] Registrar intervalos de confianza y muestras suficientes para percentiles.
- [ ] No trasladar cifras DGX a RTX Spark como medición propia.
- [ ] No convertir petaflops/TOPS en tokens/s mediante equivalencia universal.
- [ ] Probar fallos de GPU, OOM, desconexión, crash y reinicio del host.
- [ ] Validar pause/cancel/unload: congelar no equivale a liberar VRAM.
- [ ] Ejecutar soak tests de jornadas completas con workloads reales.
- [ ] Definir umbrales de aceptación por perfil con el operador.
- [ ] Publicar resultados negativos y limitaciones junto a resultados favorables.

## 11. Installer, operación y soporte — P1/P2

- [ ] Auditar wizard y preflight existentes antes de extenderlos.
- [ ] Detectar entorno y presentar capacidades verificadas antes de instalar.
- [ ] Mostrar degradaciones y backend elegido con explicación.
- [ ] Descargar artefactos por versión/hash y origen autorizado.
- [ ] Mantener credenciales fuera de planes, logs y workers innecesarios.
- [ ] Instalar identidades, ACL y servicios mínimos por plataforma.
- [ ] Iniciar sin terminal visible ni interacción permanente en el host.
- [ ] Exponer métricas, inventario, errores y fallback desde ArchieOS.
- [ ] Mantener notebook/cliente remoto como administrador y observador.
- [ ] Probar rollback de runtime, driver compatible y modelos/adaptadores por separado.
- [ ] Probar desinstalación sin borrar proyectos o datos del operador.
- [ ] Mantener diagnósticos exportables sin secretos ni payloads privados.
- [ ] Crear locks de dependencias por plataforma y planes para drivers legados.
- [ ] Ofrecer procedimiento offline y recuperación sin acelerador.
- [ ] Documentar soporte comunitario/experimental frente a soporte validado.

## 12. Clasificación de soporte y Definition of Done

Cada combinación se clasifica por capacidad, nunca por marketing:

| Estado | Significado |
|---|---|
| No evaluado | No existe evidencia suficiente |
| Investigado | Documentación/código identificado; no implica ejecución |
| Experimental | PoC real con limitaciones y sin garantía operacional |
| Validado | Pruebas de capacidad y seguridad en revisión y hardware registrados |
| Operacional | Instalación, reinicio, convivencia y recuperación verificados |
| Degradado | Funciona con limitaciones conocidas y explícitas |
| No soportado | Restricción demostrada y alternativa documentada cuando exista |

- [ ] Registrar SO/arquitectura/GPU/driver/runtime/modelo por resultado.
- [ ] Adjuntar comando, logs sanitizados, fecha y revisión del código.
- [ ] Exigir prueba de autoridad y permisos además de rendimiento.
- [ ] Exigir prueba de instalación y recuperación para declarar operacional.
- [ ] Exigir prueba de convivencia para perfiles «juego + Archie».
- [ ] Revalidar resultados cuando cambie una dependencia material.
- [ ] Etiquetar estimaciones y mediciones; no marcar una tarea completa por una propuesta.
- [ ] Asignar responsable y siguiente prueba a cada gap.
- [ ] Mantener esta matriz junto a los documentos de estado, sin reemplazar su evidencia histórica.

## Orden de ejecución recomendado

1. Reconciliar checkouts y capacidades existentes; elegir fuente canónica.
2. Preservar invariantes y preparar matriz de pruebas reproducibles.
3. Validar PC NVIDIA x86_64 disponible y CPU fallback como primeras referencias.
4. Validar entrega de proyectos y control host autorizado existentes.
5. Extender perfiles independientes a Apple Silicon/Metal, AMD Halo y ARM Spark.
6. Validar Titan, RTX 2080 y GPUs de servidor por arquitectura y driver exactos.
7. Integrar installer, convivencia, operación continua y clasificación de soporte.

No se selecciona un driver, bridge o reestructuración por anticipado. Cada integración necesita evidencia, revisión de contratos y un plan propio antes de implementarse.
