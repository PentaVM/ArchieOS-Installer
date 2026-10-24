# Wanda TRAIN 004: cuatro familias nuevas, revisión del autor

2026-10-16. Autor real de este lote: `gpt-6.1-sol`; procedencia owned-synthetic, revisión pending-human. Repo origin verificado PentaVM/ArchieOS-Nvidia; HEAD leído `652ce23`. Se leyeron weekly LOG, STEERING y revisión propia de round2; no se inspeccionaron ítems de benchmarks ni familias del anchor development. Claude sigue siendo main dev. Solo se escribieron este informe y el candidato de round3. Sin commit, push, control, worker, entrenamiento, modelos, tokenizador, runtime nativo, Torch ni GPU.

El LOG informa que la nueva sonda prompt-only no encontró ganancia decisional en los adaptadores anteriores y que fallaron los tres contrafactuales de acción del ancla. Ese resumen orienta a intervenciones causales; no se copiaron sus ítems ni se usaron para redactar errores equivalentes. Este lote es generación de contenido, no resultado de mejora del modelo.

## Revisión causal antes de congelar

Cuatro TRAIN: dos work con cuatro opciones cada una; dos personal con tres. Se generan 16 variantes por seed. Cada contrafactual cambia una sola evidencia; permanecen objetivo, horizonte, situación y alternativas. Los cuatro requieren cambiar acción. No hay etiquetas morales ni de rechazo de contenido: no contratar en agencia es una decisión económica pedida por el propio usuario.

| Familia | Mecanismo/ancestro | Base → contrafactual; dato decisivo y costes |
| --- | --- | --- |
| r3-agencia-salida | `retained-exit-option` / `owned-r3-agencia-salida` | B→A. Una cláusula exige pagar los tres meses o permite salir sin pagar meses no usados. El usuario condiciona contratar a esa salida, admite explícitamente no contratar y prefiere hacerlo hoy si cumple. La otra agencia tampoco permite salir; hoy no se negocian excepciones. La comisión del recomendador no demuestra engaño ni sustituye el contrato. Caminarse de la negociación gana en base; firmar gana al existir la salida requerida. Se reconoce pérdida de campaña y molestia de las contrapartes. |
| r3-asesor-incentivo | `paid-signal-value-of-verification` / `owned-r3-asesor-incentivo` | A→B. Solo cambia régimen de remuneración, fijo frente a comisión por M. Tasas de acierto explícitas por régimen: 90 % frente a 60 %, error de coste 100. Seguir cuesta 10 frente a 40 esperados; verificar cuesta 20 con precisión fijada. Cancelar 35 y aplazar+medir 50. Se recomienda actuar o verificar según esos valores, sin convertir conflicto de interés en certeza de mentira. Costes relacionales de desconfianza, pérdida de venta y reorganización son separados del coste monetario del objetivo. |
| r3-alias-fecha | `timestamped-preference-supersession` / `owned-r3-alias-fecha` | A→B. Mensaje directo auténtico a las 18:00 pide Bruma; captura auténtica pide Cobre a las 17:00 o 19:00. Solo cambia la fecha original de la captura. Reenviarla el miércoles no actualiza el deseo. La imprenta no espera otra consulta y admite un solo alias: aplazar pierde impresión hoy. No se inventa un cambio de objetivo ni se presume falsificación. |
| r3-dueto-confirmacion | `public-acknowledgement-second-order` / `owned-r3-dueto-confirmacion` | B→A. Ambos están listos en ambos mundos. En base cada uno recibió confirmación ajena, pero no sabe si su propia confirmación llegó al otro; en CF ambos vieron juntos recepción mutua. La condición explícita es saber que el otro recibió su confirmación, no simplemente saber que está listo. Pantalla compartida establece ese nivel; una nota privada a uno no lo hace para ambos. Confirmar añade diez segundos y puede ser redundante en CF; actuar antes de cerrar el conocimiento puede romper la coordinación acordada. |

Antecesores declarados en provenance son grupos propios de autoría, no una prueba automática de originalidad. Ninguna es derivada textual de round2. Los mecanismos conectan conceptualmente con el corpus: incentivo de asesor comparte familia amplia de credibilidad; dueto comparte acceso con cabina/ensayo, pero añade recepción observada de segundo orden; alias trata orden temporal de dos preferencias auténticas, no falta de acceso a una actualización. Agencia trata valor de conservar salida futura, distinto del mandato de un representante o cobertura monetaria de un fallo. Estas conexiones deben conservarse como grupos amplios de mecanismo al analizar diversidad; no afirmar independencia temática total por tener IDs distintos.

Se revisaron consecuencias en ambos mundos: pagos de agencia dependen de cláusula, riesgo de asesor depende de régimen, alias correcto depende de orden temporal y pausa de dueto depende de recepción mutua. Los hechos que cambian la elección están en el prompt: tasas, costes, límites de cancelación, próxima respuesta de P1, un solo alias, canales y duración de avisos. No se introdujeron hechos decisivos solo en justificaciones. Incertidumbre explicita aplicabilidad de auditoría, comportamiento real de agencia, preferencias no comunicadas y pérdida posterior de conexión. Las probabilidades de asesor son supuestos sintéticos dados por el escenario, no datos empíricos medidos ni calibración del modelo.

Hay decisiones firmes/rápidas y una salida justificadas por objetivos concretos, sin regla de que conciliar o esperar gane siempre. Algunas alternativas siguen dominadas por límites explícitos; cuatro familias no bastan para demostrar discriminación social difícil. Las longitudes se redactaron con naturalidad antes de computar baselines; no se retocaron etiquetas para mejorar resultados. La auto-revisión no reemplaza revisión humana ni independiente.

## Comprobación CPU sobre el candidato

Se ejecutaron los módulos reales vigentes `archie_wanda.data` y `archie_wanda.metrics`: load_families, build_examples y render; se parsearon targets JSON. Con seeds de variante 17 y 29 pasaron IDs únicos, esquema, 4 train, dominios 2/2, dos familias de cuatro opciones, una sola sustitución sin adiciones, referencias/nombres resueltos, igualdad de clave original clean/noise/cosmetic y cambio de clave original en los cuatro CF. No se importó Torch. No se midieron tokens, ventana, pérdida, generación ni rendimiento de ningún modelo.

Baselines de la implementación actual: leen únicamente acciones mostradas, longest/shortest con desempate de letra mostrada alfabética; always_A; random muestreado con seed fijo 20261016 y orden de ejemplos existente. La expectativa uniforme analítica es (2×1/4+2×1/3)/4 = 29,1667 % por variante, no 33,33 % para todo el lote. Random muestreado es una realización pequeña, no esa expectativa.

SHA256 congelado: `557cea647e785de1ae64778b03ffa0260b31fe66ca747618b152557d4eabd8bd` (16569 bytes).

| Seed variantes | Baseline | clean | noise | cosmetic | CF | Total | Familias 4/4 | Flip CF |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 17 | longest_option | 25.0 % | 25.0 % | 50.0 % | 25.0 % | 31.25 % | 0.0 % | 0.0 % |
| 17 | shortest_option | 25.0 % | 25.0 % | 25.0 % | 0.0 % | 18.75 % | 0.0 % | 0.0 % |
| 17 | always_A | 25.0 % | 25.0 % | 50.0 % | 50.0 % | 37.5 % | 0.0 % | 0.0 % |
| 17 | random | 100.0 % | 50.0 % | 25.0 % | 50.0 % | 56.25 % | 25.0 % | 75.0 % |
| 29 | longest_option | 25.0 % | 25.0 % | 50.0 % | 25.0 % | 31.25 % | 0.0 % | 0.0 % |
| 29 | shortest_option | 25.0 % | 25.0 % | 25.0 % | 0.0 % | 18.75 % | 0.0 % | 0.0 % |
| 29 | always_A | 25.0 % | 25.0 % | 25.0 % | 50.0 % | 31.25 % | 0.0 % | 0.0 % |
| 29 | random | 100.0 % | 50.0 % | 0.0 % | 50.0 % | 50.0 % | 0.0 % | 75.0 % |

Los porcentajes son sobre cuatro familias por variante y dieciséis ejemplos totales, con dependencia entre variantes. Un flip de una regla sin leer evidencia no acredita sensibilidad causal correcta. Las reglas de longitud y letra fija mantienen sus acciones entre clean/CF porque los textos de opciones son iguales, de modo que no pueden acertar ambos mundos de una familia con acción distinta. No modificar claves para reducir una baseline alta de un subconjunto; revisar artefactos de autoría y ampliar evidencia antes de inferir generalización.

## Caracteres, no tokens

Renders reales con seed 17, incluido system y marcadores:

| Ejemplo | Prompt | Target | Completo |
| --- | ---: | ---: | ---: |
| r3-agencia-salida:clean | 1375 | 1415 | 2800 |
| r3-agencia-salida:noise | 1620 | 1415 | 3045 |
| r3-agencia-salida:cosmetic | 1369 | 1412 | 2791 |
| r3-agencia-salida:counterfactual | 1374 | 1375 | 2759 |
| r3-asesor-incentivo:clean | 1350 | 1325 | 2685 |
| r3-asesor-incentivo:noise | 1566 | 1325 | 2901 |
| r3-asesor-incentivo:cosmetic | 1347 | 1325 | 2682 |
| r3-asesor-incentivo:counterfactual | 1353 | 1363 | 2726 |
| r3-alias-fecha:clean | 1328 | 1073 | 2411 |
| r3-alias-fecha:noise | 1564 | 1073 | 2647 |
| r3-alias-fecha:cosmetic | 1323 | 1069 | 2402 |
| r3-alias-fecha:counterfactual | 1328 | 1097 | 2435 |
| r3-dueto-confirmacion:clean | 1424 | 1168 | 2602 |
| r3-dueto-confirmacion:noise | 1647 | 1168 | 2825 |
| r3-dueto-confirmacion:cosmetic | 1422 | 1168 | 2600 |
| r3-dueto-confirmacion:counterfactual | 1409 | 1156 | 2575 |

Estos conteos no certifican caber en 1024 tokens. El encargado de preparación debe medir con el tokenizer fijado sin truncar. Mantener el candidato fuera de datos admitidos hasta revisión independiente/humana y revisión de ventana. Ninguna variante debe usarse como evaluación independiente de sus familiares. El lote no modifica ni demuestra la meta semanal, la calidad decisional del adaptador o resultados FANToM/OpenToM.
