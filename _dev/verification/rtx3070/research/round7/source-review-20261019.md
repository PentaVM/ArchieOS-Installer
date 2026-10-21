# Fuentes propuestas por el propietario: auditoría CPU

Se revisaron después de integrar el diseño PR41, antes de admitir o encolar
entrenamiento. `audit_sources_cpu.py` y `source-audit-20261019.json` conservan
revisiones y hashes, sin copiar conversaciones ni transcripción al corpus.
Descarga acotada: JSON de5.21MB y dos respuestas pequeñas del visor. Sin modelos,
GPU, cambios de etiquetas, cola, control, locks ni procesos de Claude.

## samhog/psychology-10k

[Repositorio primario](https://huggingface.co/datasets/samhog/psychology-10k),
revisión `2d60e5006b2680ee30e8707da4ca763ccb2914f9`.
Se auditó el JSON completo:9846filas con `input`, `output`, `instruction`.
Hay6579consultas normalizadas distintas y3267ocurrencias posteriores a la
primera;1017consultas tienen más de una respuesta distinta. No hay pares
consulta/respuesta normalizados exactamente duplicados. Estas cifras miden
repetición de consultas, no contradicción clínica ni contaminación de una
partición actualmente usada: no se creó tal partición.

La misma instrucción de rol profesional aparece en todas las filas. Sus salidas
son respuestas de orientación conversacional; faltan objetivos del host,
alternativas, acciones gold, variables causales, contrafactuales, dominios y
ancestría de familias. Convertir automáticamente esas respuestas en seis campos
inventaría supervisión que el origen no certifica. Entrenarlo directamente puede
cambiar el estilo/rol en lugar de mejorar la decisión causal; es un riesgo de
ajuste de tarea, no una mejora o regresión medida.

No hay dataset card, LICENSE ni campo de licencia en los metadatos consultados;
el repositorio enumera solamente `.gitattributes` y el JSON. Tampoco se documenta
ahí revisión profesional, generación, consentimiento o procedencia original.
La ausencia de esa información no demuestra que el material sea ilícito, real
o sintético: deja esos aspectos sin verificar. No se admite como TRAIN crudo.

## Gopher-Lab / episodio de Becky Kennedy

[Repositorio primario](https://huggingface.co/datasets/Gopher-Lab/huberman_lab_Dr__Becky_Kennedy_Protocols_for_Excellent_Parenting__Improving_Relationships_of_All),
revisión `63cb615b31cc9e4c4d410d10570ef9ef94435223`.
README fijado a esa revisión; se inspeccionaron las163filas del visor, sin
truncamientos, y la revisión se comprobó igual antes/después. El visor no se
presenta como un fichero Parquet fijado: se conservan hashes de sus respuestas.

Es una transcripción con `speaker` y `text`:82turnos A,81turnos B y177125caracteres.
Ocho textos normalizados repiten otro texto; podrían ser repeticiones normales
del diálogo, no errores. El segmento mayor tiene7877caracteres: caracteres no
equivalen a tokens. No se afirma fit sin tokenización y contexto conversacional.
El episodio completo es un solo grupo de origen: repartir sus turnos al azar
entre TRAIN y desarrollo produciría dependencia entre ambos conjuntos.

La card sólo contiene nombre y metadatos estructurales, sin licencia, enlace de
procedencia original, método de transcripción o supervisión de respuestas.
No hay objetivos/acciones gold ni contraste causal. Puede orientar una futura
curación temática, pero esos turnos no equivalen a163familias independientes
ni a datos listos para decision_v1. No se encola el texto crudo.

## Entrega accionable

1. Resolver procedencia/licencia y documentar qué se autoriza a reutilizar.
2. Elegir mecanismos pertinentes; redactar familias originales con objetivos
   explícitos, evidencia diferenciada de declaraciones, costes relacionales,
   incertidumbre y contrafactual de una sola causa. No convertir consejos de
   autoridad en etiquetas gold ni añadir moderación moral.
3. Conservar autoría/episodio como grupo, separar familias y dominios, revisar
   gold previo a `selected_option_id`, sesgos de longitud y fit completo.
4. Mantener toda autoría sintética nueva `pending-human`. Comparar sólo una
   tanda revisada con presupuesto de pasos y exposición comparable, gates
   numéricos/export nativo y coordinación real antes de un job GPU acotado.

No se generaron más filas ni se encoló un trabajo de entrenamiento. Ya hay20
familias TRAIN propias/80variantes revisables en round2/round3, con fit nativo
documentado en round6. Priorizar su admisión o una ablación informativa sobre
añadir volumen externo sin supervisión. Esto no cambia el material evaluation-only
FANToM/OpenToM ni incorpora sus ítems.
