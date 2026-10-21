# Derivadas compactas opcionales: bono y cabina

Estado: **candidatas opcionales, pending-human; requieren revisión independiente y medición real de tokens**. No son dos nuevas familias independientes, no son evaluation y no deben sumarse a los padres como muestras independientes. Solo se crearon este informe y `curriculum-003-compact-candidate.json`. No se editó el snapshot aprobado de 16 familias ni producto, control, modelos o recibos.

Padre: `curriculum-003-candidate.json`, SHA256 `98c9929a3b8836bd25b422d499f9648933c5577084cac673f859e5ffd5ae5461`.
Derivada congelada: `curriculum-003-compact-candidate.json`, SHA256 `a6eff2ddebbe2755c4937a3da9f4aea1737738d49496b9070fc286edeac9202f`.

IDs: `r2c-bono-metrica` deriva de `r2-bono-metrica`; `r2c-cabina-interprete` deriva de `r2-cabina-interprete`. Cada provenance conserva `owned-synthetic`, autor `gpt-6.1-sol` y `pending-human`, e incluye explícitamente `parent_family_id` y `parent_sha256`. Deben agruparse por padre causal para cualquier muestreo o split; este lote permanece exclusivamente TRAIN.

Se acortaron situación, evidencia, acciones y targets con español conciso; se conservaron textualmente objetivo, horizonte, dominio, split, stratum, variables causales, nombres y renombres. Se mantuvieron las tres opciones y las mismas etiquetas base/contrafactual. Se conservó el único reemplazo causal: verificación independiente del conteo frente a conteo exclusivo del organizador; o falta de recepción del cambio de glosario frente a recepción y confirmación escritas. Las cuatro variantes las produce el código existente, sin quitar ruido global ni estructura de decision_v1.

Auto-revisión de contenido: bono mantiene 800 fijo, 600+500 a veinte asistentes, 70 % basado en reservas auditadas, materiales 100, pérdida de medio bono bajo conteo exclusivo basada en cuatro encargos comparables, idéntico contenido/fechas, y justificaciones de netos esperados 850/675 frente a 700. Cabina mantiene quién corrigió el término y cuándo, registro de recepción, uso de último glosario, confirmación escrita antes de traducir, nota de 30 segundos, guion de cuatro minutos ya preparado y condición de revisión por cambios adicionales o confirmación fallida. Consecuencias siguen condicionales entre mundos. Las opciones de 950 y de repetir el guion conservan sus costes y su menor atractivo; la compresión no demuestra mayor dificultad social.

La revisión es del autor y no acredita equivalencia semántica independiente. La incertidumbre de bono sigue siendo textual, no un pronóstico calibrado. Cambiar ID cambia determinísticamente permutación y ruido seleccionado por el renderer; no crea nueva evidencia independiente.

## Comprobación CPU, sin tokenizer

`load_families`, `build_examples(...,17)` y `render` vigentes pasaron para dos familias work/train y ocho ejemplos. Se parsearon los ocho targets JSON, se comprobó prefijo del prompt, referencias y nombres resueltos; por comparación con el padre se verificaron igualdad de objetivos/horizontes/selecciones e intervención sobre la misma clave. No se importó Torch. No se invocaron GPU, runtime nativo, tokenizer ni modelo. El SHA256 del padre siguió igual después de escribir derivados.

| ID | Variante | Caracteres prompt | Caracteres target | Caracteres render completo |
| --- | --- | ---: | ---: | ---: |
| r2c-bono-metrica | clean | 1090 | 799 | 1899 |
| r2c-bono-metrica | noise | 1243 | 799 | 2052 |
| r2c-bono-metrica | cosmetic | 1088 | 797 | 1895 |
| r2c-bono-metrica | counterfactual | 1086 | 808 | 1904 |
| r2c-cabina-interprete | clean | 1051 | 819 | 1880 |
| r2c-cabina-interprete | noise | 1236 | 819 | 2065 |
| r2c-cabina-interprete | cosmetic | 1047 | 819 | 1876 |
| r2c-cabina-interprete | counterfactual | 1067 | 818 | 1895 |

Son caracteres Unicode medidos sobre los strings reales, incluidos system, marcadores y terminación; **no son tokens ni una prueba de caber en 512**. La variante noise conserva los dos ruidos globales añadidos por el código además del ruido familiar. El agente encargado debe medir los ocho renders con el tokenizer previsto antes de cualquier selección para un entrenamiento de ventana 512; no truncar evidencia o targets para forzar admisión. Si caben, siguen pendientes de revisión independiente y deben sustituir opcionalmente a sus padres en ese experimento, no contarse dos veces.
