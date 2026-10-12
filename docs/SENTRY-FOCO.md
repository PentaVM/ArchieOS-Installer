# Feeder unificado y perfiles de foco

`archie_cerebra::feeder` es el único camino de las fuentes de Sentry y Wanda hacia
`PillarContext` y Cerebra. Los productores no cambian: siguen validando, sellando en el
Vault a `restricted` con su propia hora y deduplicando por referencia derivada del
contenido.

| Fuente | Productor | Pilar | Peso (criterio, no medida) |
|---|---|---|---|
| Alertas de netwatch | `sentry_bridge::record_alerts` | Sentry | notice 30, warning 60, high 90; hipótesis = mitad |
| Cambios de dispositivo | `feeder::record_device_events` (nuevo) | Sentry | aparece 50, cambia de dirección 40, desaparece 20 |
| Mensajes de chat (exports propios) | `wanda_bridge::record_chat_context` | Wanda | saliencia de Wanda |
| Notificaciones (dispositivos propios) | ídem | Wanda | saliencia de Wanda |
| Agregado por chat | ídem | Wanda | 10 por mensaje, tope 100 |

Cada registro se convierte en un `FeedItem` (referencia, fuente, hora propia, peso y
categoría dominante). `feeder::plan` aplica un `FocusProfile` y devuelve qué entra, en
qué orden (prioridad, peso, recencia) y qué se descarta y por qué (`NotInProfile`,
`BelowWeight`, `Category`, `OtherDomain`, `Budget`, `Duplicate`). `consider_plan` corre un
pase de Cerebra.

## Perfiles

| Perfil | Dominio | Entra | Tope |
|---|---|---|---|
| `pantalla-1` | cualquiera | alertas ≥ warning, cambios de dispositivo ≥ 40. Nunca chats ni notificaciones: la pantalla la ven otros. | 8 |
| `trabajo` | sólo `work/…` | mensajes de categoría trabajo/urgencia/pregunta/planificación, notificaciones de trabajo o urgencia, agregados; alertas sólo `high` | 24 |
| `personal` | sólo `personal/…` | mensajes, notificaciones con saliencia ≥ 10, agregados; alertas sólo `high` | 32 |
| `red` | cualquiera | todas las alertas y todos los cambios de dispositivo; nada social | 64 |

## Lo que un perfil no puede hacer

- **Cruzar dominios.** Un plan tiene un solo `ContextDomain`; lo que esté fuera se descarta
  como `OtherDomain`. `trabajo` no se planifica en un dominio personal ni `personal` en uno
  de trabajo (`FeederError::ProfileDomain`).
- **Bajar la graduación.** La observación del pase sale `restricted`. Sin modelo local
  permitido, el pase se rechaza (`NoLocalModel`) y ese rechazo es el resultado.
- **Inventar contenido.** Sólo referencias a entradas que ya existen.

## Uso

```sh
archie focus pantalla-1
archie focus trabajo --domain work/pentavm/archie --json
archie focus red --since-ms 1760000000000
```

`archie focus` lee el Vault que mantienen `sentry-follow` y `wanda-watch`. Para pesar cada
entrada la tiene que leer: cada lectura queda en el registro de auditoría del Vault con el
actor `focus-feeder`. El camino en memoria (`FeedItem::from_alert` y demás) no lee nada.

## Evidencia

Pruebas en `core/crates/archie-cerebra/tests/feeder.rs` con fixtures (direcciones de
hardware sintéticas, mensajes escritos para la prueba). Un humo con el binario real
(`wanda-ingest` de un volcado de notificaciones y luego `archie focus`) admitió las tres
referencias esperadas y terminó en `refused:NoLocalModel`. No se ha corrido sobre datos
reales del dueño ni con un modelo local.
