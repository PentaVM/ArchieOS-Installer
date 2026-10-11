# Archie personal y laboral: integración y recorrido operativo

## Intención

El mismo Archie puede preparar un proyecto CAD y un borrador de respuesta al jefe
mientras el propietario usa el equipo. La tarea laboral usa evidencia del proyecto;
la tarea personal usa contexto personal. Comparten el OS y sus recursos, pero no
comparten automáticamente memorias, credenciales, ventanas de entrenamiento ni
adaptadores aprendidos. La revisión es sobre resultados concretos y modificables.

## Estado del cambio de 2026-10-11

Implementado en este worktree: límites personal/proyecto en Vault y Cerebra,
composición acotada de referencias de los pilares, preparación cognitiva desde
el Enterprise Bridge de Santiago, filtro del tablero por dominio, atribución
laboral de sus entradas y rechazo de una sincronización sin transporte.

No desplegado: productores de percepción, controlador autónomo de tareas,
recorder REM y transportes externos. No se acredita que todos los pilares estén
conectados en vivo. Esta implementación cierra el límite de contexto sobre el
que se deben conectar esos consumidores; el apartado de pendientes sigue vigente.

Verificación local: `cargo test -p archie-vault -p archie-cerebra -p
archie-workstation -p archie-rem -- --test-threads=1`, con compilación serial y
sin incremental: 128 pruebas pasan, 0 fallan, 1 omitida porque requiere FreeCAD
real empaquetado para Linux. Las pruebas incluyen HTTP real en loopback, pero
no sensores físicos, inferencia de pesos reales ni envío a servicios externos.

## Responsabilidades que se conservan

| Pilar | Entrada y resultado | Límite |
|---|---|---|
| Sentry | Estado del entorno, dispositivos y observaciones atribuibles | Observar no autoriza ejecutar ni transforma datos en entrenamiento |
| IRIS | Aplicación, superficies, acciones y cambios observados | La observación de una interfaz no acredita éxito del trabajo |
| Wanda | Contexto de relaciones, preferencias e hipótesis de interpretación/tono | Una hipótesis sobre el jefe sigue siendo hipótesis y puede corregirse |
| Vault | Hechos privados, documentos y credenciales por referencia | Las credenciales van al adaptador, no al prompt |
| Memoria/Focus | Objetivo, progreso, evidencia, restricciones y checkpoints | Resumir conserva procedencia, contradicciones y evidencia de respaldo |
| Cortex/Cerebra | Asesoría y secuencia de decisiones/ruteo | Sin autoridad nueva para el modelo o el host |
| Forge y Bridges | Plan técnico, operaciones profesionales, validación y exportación | Usar los adaptadores reales de FreeCAD/Blender/etc.; no inventar éxito |
| Policy/Stage/ejecutor | Capacidades, leases, operaciones y control del usuario | El contexto laboral no sirve como permiso de ejecución |
| Desk | Entregas, versiones y revisión | Un borrador no equivale a un envío por WhatsApp |
| REM | Ventanas verificadas, aprendizaje local, borrado y reanudación | Opt-in por dominio; respetar el supervisor existente y el benchmark sellado |

## Uso del contexto

El nuevo circuito utiliza `personal/<owner>` y `work/<organisation>/<project>`.
Estos son identificadores opacos. `PillarContext` agrupa referencias de la misma
tarea y dominio, con un límite de 64 referencias. Cerebra recupera contexto por
el gateway de Vault después de evaluar el floor; el gateway comprueba todos los
prefijos antes de descifrar y los recibos conservan el dominio sin texto privado.
El contexto del circuito se gradúa Restricted para que el router seleccione un
modelo local elegible. Qwen o MiMo son opciones de catálogo, no decisiones
codificadas por nombre. La elección sigue capacidades, disponibilidad y calidad.

No mover registros antiguos al nuevo namespace sin revisar su propietario,
proyecto y retención. Un dato útil para ambos ámbitos debe generar una copia
derivada mínima con procedencia mediante Core; bajar su privacidad no demuestra
que se permita compartirlo entre ámbitos.

## Ejemplo: preparar planos y una respuesta

1. El bridge empresarial registra la solicitud en cuarentena y con atribución
   laboral. Core asigna el dominio del proyecto y admite el trabajo conforme a Policy.
2. Wanda aporta preferencias de comunicación y evidencia de la relación laboral;
   Vault aporta el encargo, restricciones y versiones del proyecto.
3. Cerebra crea una cápsula de la tarea y selecciona un proveedor local. Forge
   propone operaciones que el bridge CAD y Policy evalúan antes de ejecutar.
4. IRIS observa progreso; el adaptador verifica recomputación, geometría,
   dimensiones y archivos exportados frente a criterios concretos del encargo.
5. Desk muestra archivos editables, vistas previas, cambios, comprobaciones y el
   borrador de respuesta. El propietario modifica o verifica el resultado.
6. Un adaptador de mensajería autorizado realiza el envío y devuelve un recibo.
   No se considera enviado porque se haya escrito el texto o pulsado un botón.
7. La memoria conserva hechos y recibos por referencia. REM admite aprendizaje
   solo de ventanas y etiquetas verificadas, en el dominio habilitado.

## Integración pendiente con runtime y hardware

La presencia de los tipos y pruebas no significa que los servicios estén activos.
Mantener visibles estos pendientes hasta probarlos de extremo a extremo:

- Unir el Sentry operacional de la copia Archie con la base unificada sin perder
  su journal, operador Linux, comprobaciones y permisos.
- Conectar el receptor autenticado de captura y el IPC de cámaras a Core.
- Hacer que el runtime construya PillarContext desde productores autenticados;
  un enum de fuente identifica la procedencia declarada, no la autentica.
- Resolver dominios desde sesión, aplicación, cuenta y proyecto autorizado;
  no confiar en que un modelo o webhook elija el dominio.
- Vincular esta preparación de contexto a inferencia, herramientas, comprobación,
  checkpoints y Desk mediante los servicios reales, con pruebas de fallos y revocación.
- Implementar el transporte empresarial que hoy falta y el adaptador WhatsApp;
  conservar cuarentena y destinatario/cuenta/dominio ligado al envío.
- Conectar Stage/IRIS a presence_file y al recorder REM. El recolector no puede
  fabricar el fichero de recibos de supervisión; el juez/corrección lo produce.
- Separar buffers, claves y adapters REM por dominio, con recuperación y borrado
  probados. El esquema Python estricto requiere una evolución coordinada para
  incorporar dominio, no campos adicionales ignorados o un spool alternativo.
- Habilitar modelos y backends de inferencia locales según el equipo y medir su
  residencia, latencia y consumo antes de elegir una configuración de arranque.

## Recursos y benchmark

Archie usa las clases Active/Background/DeepIdle y checkpoints existentes. REM
entrena cuando el usuario está ausente; el trabajo útil puede continuar durante
juego con presupuesto y preempción. No existe una garantía universal de rendimiento
simultáneo para todos los juegos, modelos o GPUs.

Medir con versiones, modelo/digest, cuantización, contexto, backend y hardware
registrados: arranque frío/caliente, TTFT p50/p95, tokens/s, latencia de acción,
exactitud de verificaciones, RAM/VRAM pico, consumo y tiempo de recuperación.
Comparar juego solo, juego con observación, juego con inferencia y juego con CAD;
registrar FPS/1% lows y frametimes p95/p99. Probar retorno del usuario, presión de
memoria, caída del proveedor, pérdida de sensores y revocación a mitad de tarea.
El ahorro de trabajo se mide en minutos del propietario y correcciones necesarias,
no se fija de antemano en 99%.
