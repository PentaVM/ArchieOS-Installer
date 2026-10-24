# Cierre del apoyo CPU a Claude

Ventana autorizada hasta2026-10-16 16:08UTC (13:08America/Santiago). Cierre en el
primer heartbeat posterior,16:13UTC. La automatización de Codex
`wanda-rtx3070-supervisi-n-de-diez-horas` quedó **PAUSED** mediante la herramienta
de la app. No se prolongó el apoyo ni se detuvo/modificó la tarea de Claude.

Claude sigue siendo el main dev. Durante la fase de apoyo CPU, Codex no cargó
Torch, modelos, tokenizadores nativos o CUDA, ni modificó trainer, cola,
control.json o watchdog. La pausa anterior a esta fase está documentada aparte.
El archivo ajeno sin seguimiento `training/wanda_bonsai2/ops/thermal-guard.sh`
se preservó.

## Entregables de Codex

- Auditoría reproducible de40familias/160variantes: baseline de opción más larga
  25/32en development y102/128en train; todos los targets aportan consecuencias
  gold antes de la fila de selección teacher-forced. No son benchmarks de Wanda.
- Revisión de fuentes primarias y sus hashes; prioridades CaSiNo/SODA/ATOMIC2020/
  GoEmotions y separación estricta de FANToM/OpenToM como evaluación. Las dos
  peticiones de muestras HF devolvieron502; cards y metadatos sí se obtuvieron.
- Informe `2026-10-16-claude-cpu-advice.md` con propuestas de prompt-only,
  contrafactuales, control de longitud, ancestry/splits, experimentos y calibración.
- Addendum persistido en la tarea de Claude, con backup previo y recibo de hashes.
  La lectura efectiva de ese addendum por el scheduler de Claude no fue observada.
- Publicación en PR38, commit5eb03b0; sin merge ni intervención en GUI/WSL/Core.

## Recibos nuevos encontrados al cerrar

La revisión periódica de los resultados publicados en Git no mostraba nuevos
informes de Claude. Al cerrar se leyó además el directorio vivo
`/home/archie/wanda/run/runs` y se encontraron estos cuatro recibos. Se copiaron
sus bytes JSON a `overnight-receipts/`, sin copiar ni cargar checkpoints.
Son reportes producidos por el worker de Claude, no una ejecución independiente
de Codex ni una verificación de calidad o hardware por parte de este análisis.

| Run | Pasos reportados | Loss base→final | Choice base→final | Family-all-correct base→final |
| --- | ---: | --- | --- | --- |
|w1-qv-s17|48|2.438934→2.074374|0.90625→0.90625|0.625→0.625|
|w1-qv-s29|48|2.438934→2.068572|0.90625→0.90625|0.625→0.625|
|w1-qkvo-s17|48|2.438934→1.799424|0.90625→0.90625|0.625→0.625|
|w1-qv-lr2e4-s17|48|2.438934→1.691254|0.90625→0.90625|0.625→0.625|

Los cuatro indican `schedule_complete`. Counterfactual accuracy permanece0.625
y flip-rate0; choice/family/noise/cosmetic no mejoran en los recibos. La loss
menor demuestra únicamente mejor ajuste bajo ese scoring; no mejora estratégica
verificada. El scoring teacher-forced ya señalado impide interpretar90.625% como
resultado de una evaluación ciega desde el prompt.

La pareja q/v seeds17/29 muestra el mismo resultado discreto; q/k/v/o o mayor lr
reducen más la loss sin mejorar esas decisiones. Recomendación para Claude:
priorizar evaluación prompt-only y calidad de datos antes de elegir más capas,
más pasadas o el checkpoint con menor loss. Preservar los recibos anteriores,
el ancla de8familias y reportar una prueba nueva por separado.

## Estado y pendientes

La última lectura de `worker.json` registra `paused`, jobnull, PID20 y nuevo
boot_id5959179a-0bb3-4c58-90af-af826c4b0cf5. Esto difiere del PID23703 anterior;
no inferimos quién reinició o reanudó, ni cuándo se ejecutaron los cuatro runs.
No se cambió ese estado. El cutoff propio del worker sigue16:30UTC.
La disponibilidad actual de la GPU no se comprobó en esta fase CPU.

Persisten las revisiones de paridad original versus tolerancia revisada, la
evaluación sin gold, la revisión humana de anotaciones y la paridad del adapter
nativo. No hay resultados aquí contra los tres modelos solicitados, evaluación
sellada ni evidencia de superioridad social. Claude conserva la decisión técnica
y la ejecución bajo las autorizaciones existentes.
