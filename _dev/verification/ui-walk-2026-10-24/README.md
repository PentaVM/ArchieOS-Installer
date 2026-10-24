# Recorrido de la GUI del shell — 2026-10-24

`archie-display --shell --ui-walk display/native/tests/ui-walk-shell.json` contra el nodo de
demostración vivo (archied, displayd, worldd), en WSLg con llvmpipe.

El recorrido pasó por el dock y sus menús, Vault, Forge (con Firefox), Sentry, Ajustes,
todos los menús de la barra, la paleta, el Centro de control (con pausa y reanudación),
Actividad, el dominio, las notificaciones, el inspector y el selector de ventanas.

Cada clic guarda una captura y el estado del shell antes y después. Está en
`report.jsonl`; las capturas completas (184) quedaron fuera del repo, en
`D:\ArchieLocal\ui-walk-20261024`.

Resultado: 293 clics, de los cuales 60 quedaron sin efecto visible y 10 sin destino.
Los 70 están clasificados así; ninguno es un control de mentira:

| Casos | Qué fue |
|---|---|
| 43 | Un nuevo clic en la sección de Ajustes que ya estaba abierta: el «reset» del propio recorrido. |
| 12 | La opción que ya estaba elegida: Inicio en Vault, la fila ya seleccionada, la vista de lista activa, Oscuro, Vívidos, etc. |
| 10 | `not_found`: índices más allá del final de una lista que el recorrido había contado en otra sección. |
| 3 | Entradas de menú dibujadas sobre la barra de título de una ventana: Ir → Documentos, Vista → Movimiento reducido y Archivo → Abrir seleccionado (esta última además deshabilitada sin selección). Fallaban solo con los clics que el recorrido manda directo a la ventana. Por la entrada de Qt, que es por donde pasa un mouse, sí funcionan; `tst_MenuOverWindows.qml` lo prueba. |
| 2 | Botones deshabilitados: Descartar todos y Mostrar descartados, sin avisos que descartar ni restaurar. |

El recorrido no llegó a las opciones de Ajustes que quedan bajo el pliegue.
`tst_SettingsOptions.qml` clickea cada opción del orbe y de la apariencia y lee cada una
de vuelta.

## Capturas

- `forge-firefox-abierto.png`: Forge lanzando Firefox desde su catálogo.
- `pausa-de-emergencia.png`: el Centro de control con la pausa de Core activada (`archie ai pause`).
- `ajustes-modelos-locales.png`: Ajustes → Modelos con el modelo local que responde. Es una
  segunda pasada, después de instalar el `archie-local` con `list --json`. En la primera el
  panel decía «archie-local no listó modelos», porque en el nodo seguía el `archie-local`
  anterior.
