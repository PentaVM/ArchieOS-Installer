# Wanda — continuación autorizada, ronda2

El propietario reactivó la tarea sin fecha final: más contenido y revisión de
calidad, búsqueda continua de mejores parámetros, comentarios en PR38 para Claude,
GPT6.1Sol medio para investigación y GPT6.1Sol alto para fine-tuning propio.
Claude continúa como main dev. Esta nueva autorización permite preparar un run
propio, sin interferir con los procesos o archivos activos de Claude.

La automatización existente `wanda-rtx3070-supervisi-n-de-diez-horas` fue reactivada
como `Wanda — investigación y entrenamiento continuo`, cada30minutos, sin el cutoff
antiguo. Búsqueda periódica continua no significa GPU ejecutándose sin límite:
cada run conserva límites, checkpoints y puertas de calidad/recursos.

## Trabajo en curso

- `social_data_round2`: agente **gpt-6.1-sol / medium**, autoría de16familias
  candidatas españolas,8work/8personal, fuente propia y revisión pending-human.
  Archivos asignados: curriculum-003-candidate.json y data-review.md.
- `finetune_high_audit`: agente **gpt-6.1-sol / high**, diagnóstico numérico y
  preparación de fine-tuning. Worktree aislado:
  `C:\Users\Penta\.codex\worktrees\wanda-strict-reference\ArchieOS`, ref16841cf.
  No editar el trainer compartido. Informe asignado: trainer-audit.md.
- Root coordina, revisa datos, integra solo cambios verificados y publica
  comentarios en PR38. No se afirma que el modelo del scheduler mismo haya sido
  cambiado; las asignaciones de agentes sí especifican esos modelos/esfuerzos.

## Evidencia y restricciones actuales

El worker anterior informa stopped/cutoff. GPU medida al iniciar esta ronda:
RTX3070,704MiB usados/7315libres,25°C,17.47W. El resumen de Claude documenta
85–87°C durante entrenamiento y guard térmico entre pasos; no basta que esté fría
en reposo. No se cambian fan, potencia o driver ni se matan procesos ajenos.

Sol alto identificó controles nativos de precisión que requieren verificación:
`GGML_CUDA_CUBLAS_COMPUTE_TYPE=f32` y
`GGML_CUDA_PTQ1_0_MMQ_MAX_BATCH=0`. El prefill de más de4tokens puede usar FP32;
los kernels de1–4tokens siguen Q8. Una prueba de prefill estricta no demuestra
paridad de generación autoregresiva. No usarla para relabelar caches Q8 o declarar
la integración completa verificada. Mantener RMS<=0.0005 y KL<=0.001 originales.

Solo se autorizó al agente alto una prueba pública corta, después de tests CPU,
con guard del proceso propio: temperatura inicial<60°C, abortar>=70°C, timeout
180s y nunca TorchCUDA simultáneo con el proceso nativo. Entrenamiento semántico
espera revisión de datos, evaluación sin gold y puertas de paridad/recursos.

Auditoría previa sigue vigente: opción más larga25/32development; la elección
teacher-forced ve consecuencias gold y no es evaluación ciega. Los4runs previos
bajan loss sin cambiar choice0.90625 ni family0.625. El objetivo es mejorar
decisiones, no optimizar estas métricas infladas.

Comentarios completos y resultados se publicarán en este directorio y PR38;
los pesos, caches y datos privados no van a Git. Mantener seis campos Core,
fronteras Cortex/Policy/leases, dominios y GUI/WSL de Claude/Xavi. Sin merge ni
autopromoción; sin una nueva capa moral en Wanda.

## Avance verificado de esta ronda

Sol alto ejecutó referencias reales en el worktree aislado: tres textos públicos,
perfil strict-f32 con TF32 desactivado, todas las comparaciones con el criterio
`strict`. Suffix RMS máximo0.000180463 y KL máximo9.93e-8; microbatch nativo RMS
máximo0.000311257. El guard registró máximo50°C/6640MiB totales. Luego pasó
backward completo de248320clases con2updates mecánicos sobre35tokens, máximo
49°C/1607MiB totales. No son aún entrenamiento semántico ni paridad de generación
Q8. Los recibos originales se conservan en el worktree del agente hasta integrar.

La revisión de código independiente encontró tres defectos a corregir antes del
run propio: pin explícito de precisión en train, validación de pruebas numéricas
vacías y rechazo de NaN en comparaciones. Los recibos reales obtenidos no están
invalidados por esos defectos de admisión. Hay una ronda de corrección CPU en curso.

Los16escenarios nuevos pasaron revisión independiente después de corregir7
problemas semánticos. Snapshot candidato98c9929a3b8836bd25b422d499f9648933c5577084cac673f859e5ffd5ae5461:
8work/8personal,64variantes, pending-human. Aprobado por root solo para un pequeño
experimento sin promoción, sujeto a las demás puertas. Baseline longest12/64;
shortest24/64 y11/16en contrafactuales: limitación que se informa, no objetivo a
optimizar cambiando claves.

El primer run debe ser de un solo dominio, q/v r8alpha16, lr1e-4,2optimizersteps,
accumulation1, límites térmicos/tiempo y checkpoint. Elegibilidad: recorrer los8
work por ID y seleccionar los primeros2con las4variantes completas<=512tokens;
si solo cabe1, usar1. Solo longitud antes de cualquier salida de modelo, nunca
score de desarrollo. Si ninguno cabe, escribir una derivación compacta explícita
y revisarla, sin truncar ni relajar512silenciosamente.

## Revisión final y coordinación actualizada — 20:05 UTC

Las tres correcciones de admisión pasaron re-revisión independiente. Root ejecutó
la suite CPU: 57 tests, un hardware opt-in omitido; no se infiere hardware de esa
suite. Dos derivados compactos pasaron revisión de conservación causal. Solo
`r2c-cabina-interprete` cumple el límite completo de512tokens en sus cuatro
variantes (454/502/455/445); `r2c-bono-metrica` queda excluido.

El primer intento propio se bloqueó **antes de crear hijos o asignar GPU**:
Claude reinició su worker, PID150/boot062a7c2e-aab4-49f5-85c4-66976ecc27c3,
que conserva worker.lock. No se robó el lock ni se modificó control/cola.
El estado posterior es paused. El recibo `own-attempt-blocked.json` conserva
este intento fallido, distinto del posterior intento coordinado.

La guía y LOG de Claude, commit `c7e49db`, documentan ahora el protocolo de
compartición: Codex crea `run/hold`, espera `worker.json.state=paused`, usa GPU
y retira únicamente su propio hold. No requiere que el worker abandone su
lock de proceso. El diagnóstico propio se adapta a este protocolo con lock
exclusivo separado; no elimina un hold preexistente ni cambia los procesos
de Claude. Mantiene perfil estricto, límites70°C/7GiB/180s, dos updates y
checkpoints. El resultado efectivo se publica por separado; no se anticipa
entrenamiento, export ni mejora del modelo.

El worker vivo carga código del worktree compartido. La integración de código
nuevo se hace en el worktree aislado y en PR38, preservando los archivos que
ese proceso ya cargó. Claude decide cuándo adoptar el commit y reiniciar su
propio worker. Las recomendaciones son evidencia para verificar, no órdenes
que sustituyan al main dev.

## Reanudación de la sesión — 2026-10-17 00:42 UTC

El agente Sol alto alcanzó el límite de cuota durante el seguimiento. La adaptación
del guard existe, pero no se ejecutó un entrenamiento semántico ni se hizo commit.
Se retoma explícitamente, con prueba CPU de propiedad y orden de limpieza: los
hijos propios deben terminar antes de liberar hold, incluso al fallar.

Claude avanzó el worktree compartido hasta `652ce23`: implementó probe prompt-only
en `5c6c6af` y re-midió los cuatro adapters. Ninguna decisión cambió frente a base
(choice0.90625/family0.625/counterfactual0.625) y bajó p(correct). Los tres casos
que requieren cambio de acción fallan. Se reconoce el avance de medición; sigue
pendiente medir razonamiento autogenerado. La rama aislada estricta parte de
16841cf y modifica worker.py: requiere integración revisada que preserve ese nuevo
probe; no copiar archivos encima del código vivo.

La publicación inicial incluye datos, revisiones y evidencia investigada. Los
cambios de código aislado y sus recibos se publican por separado cuando estén
congelados; un borrador de comentario local no equivale a un comentario enviado.

## Publicación y precheck posterior

Datos, revisiones y recibos reales estrictos están en PR38 hasta `26c7456`.
Se fijó `.gitattributes` para preservar bytes de JSON: SHA256 del blob Git y
del archivo coinciden para ambos candidatos; se comprobó la cadena extraction
→suffix→backward y los límites originales.

Sol alto reanudó y pasó cinco pruebas CPU del guard de hold, incluida limpieza de
hijos ANTES de liberar la reserva. Un nuevo precheck impide el run propio:
workerPID7864 paused conserva contexto CUDA,1523MiB totales/24°C. No se creó
hold ni se inició un hijo. Se registra la dependencia para que Claude pueda
ceder contexto mediante su propio flujo cuando proceda. No se matan procesos,
no se relajan los límites y no se cuenta como entrenamiento semántico.

## Cierre del tick con continuidad programada

Comentario publicado y actualizable en
https://github.com/PentaVM/ArchieOS-Nvidia/pull/38#issuecomment-6227519798.
Código aislado publicado como propuesta `f558637` en rama
`codex/wanda-strict-f32-proposal`; PR38 contiene su patch exacto y code-handoff.md.
No se integró sobre el worker nuevo de Claude. Root repitió57tests CPU
(56pass/1hardware opt-in skip) y5tests propios de hold: pasaron.

Sol medio amplió candidatos en `../round3/`:4familias/16variantes,2work/2personal,
dos con4opciones, hash557cea647e785de1ae64778b03ffa0260b31fe66ca747618b152557d4eabd8bd.
Revisión semántica independiente sin must-fix y auditoría CPU de root pasaron;
tokens y revisión humana siguen pendientes. Total20familias/80variantes nuevas,
sin contar dos derivados de round2 como familias adicionales. No admitir ni usar
para evaluación independiente sin respetar parentesco y puertas restantes.

Siguiente tick: leer LOG/resultados y estado de los agentes antes de despachar;
comprobar si Claude cedió contexto sin interferir con él. Si no cambia, continuar
CPU con revisión útil de parámetros/datos, sin reintentos GPU ni repetir pruebas
numéricas ya pasadas. Integración del patch exige preservar el probe prompt-only
y nueva revisión, no una copia de worker.py. Automatización permanece ACTIVE
cada30minutos, sin antiguo cutoff.
