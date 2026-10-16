# Wanda PR38 — revisión del plan y evidencia real

Destino: https://github.com/PentaVM/ArchieOS-Nvidia/pull/38, rama `codex/wanda-context`; base inspeccionada72cc261c7721fa77fe8a21e25af7a55847787c74. Esta actualización publica diseño/planes y evidencia previa de compatibilidad; no afirma instalar el trainer, conectar ya todos los agentes ni entregar el canal Cortex funcionando.

## Tres mejoras incorporadas

1. **Dataset progresivo:**128familias son la primera tanda. Nuevas tandas se admiten por diversidad causal y revisión de etiquetas, con escenarios adversariales, ruido, contrafactuales, ancla de desarrollo, ablaciones y control de fuga. No hay un techo permanente de familias/libros ni crecimiento automático por pérdida de entrenamiento. Catálogo actual51títulos identificados; no51libros entrenados.
2. **AI Arena local:** registro de capacidades para Bonsai, Codex, Claude Code, IRIS y asistentes realmente descubiertos; cola autónoma acotada/reanudable. Se reutiliza Arena para tareas de escritorio/IRIS, separando comparación de modelos directos, agentes CLI y percepción/acción. Un cliente local puede usar inferencia remota. No se atribuye conversación social a un cabezal de control de IRIS ni se sustituye un modelo exacto por un agente.
3. **Cortex decision_v1:** canal estructurado de análisis inferido con alternativas, incertidumbre, consecuencias, costes relacionales y objetivos. Correlación y referencia Vault asignadas por el host, proyección Document con procedencia y lectura por contratos existentes. Core conserva sus seis campos; Recommendation/QueryResponse de este PR también se preservan. Sin autoridad adicional ni filtros morales.

Documentos:

- [Diseño del trainer](2026-10-16-wanda-bonsai2-9b-design.md) y [plan de implementación](2026-10-16-wanda-trainer-plan.md).
- [Currículo progresivo y Arena](2026-10-16-wanda-arena-curriculum-design.md).
- [Canal Cortex decision_v1](2026-10-16-wanda-cortex-decision-design.md).
- [Protocolo ciego](2026-10-16-wanda-social-benchmark-design.md) y [plan del benchmark](2026-10-16-wanda-benchmark-plan.md).
- [Catálogo de lecturas](2026-10-16-wanda-reading-catalogue.md).

## Orden para no retrasar las pruebas

Base/PTQ exacto → captura nativa completa → paridad sufijo cero/no-cero → pérdida de vocabulario completo y backward → dos actualizaciones reales con una tanda social propia pequeña → checkpoint/recarga → corrida semántica inicial acotada. Corpus ampliado, agentes CLI y transporte Cortex siguen vías independientes. No se exige esperar al corpus completo, a Claude instalado, al test sellado, a la GUI o a la nueva imagen WSL para probar esa ruta.

## Qué existe y qué falta

Probado anteriormente en la3070 con el checkpoint9B exacto: inferencia nativa CUDA; decodificación de una proyección igual a Prism; optimización LoRA de una proyección con tensores aleatorios; carga/aplicación/desactivación nativa de adapters PROBE. Esas pruebas no son entrenamiento semántico Wanda, backward del9B completo ni medición de cognición social.

Inspección actual: captura nativa exportada por símbolos C++ en la biblioteca instalada; código fuente fijado revisado para filas/máscaras/estado. La exportación del símbolo no prueba que se hayan capturado activaciones correctas. Windows tiene Codex CLI; Claude no apareció en el lookup Windows/WSL; WSL tiene `archie-arena-live`. No se ejecutaron agentes ni se comprobó su autenticación. El canal Cortex y los nuevos adaptadores Arena siguen diseñados/pendientes de implementación.

No se modifican código de producción, contratos existentes, GUI/WSL de Claude/Xavi, PR41, selección del modelo en ejecución ni pesos. No se publican credenciales, historiales personales ni capturas privadas. Las pruebas nuevas que acompañen implementaciones futuras deberán registrar resultados reales y limitaciones en esta carpeta.
