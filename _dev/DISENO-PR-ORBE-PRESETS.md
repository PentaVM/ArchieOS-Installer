# PR de diseño: presets del orbe controlados por CLI (Xavi)

Un solo PR de diseño abierto a la vez; las siguientes tandas de diseño se suman a este PR.

## Hecho
- `archie visual scene|preset|list` (archie-cli) y herramienta de voz `orb_preset`: mismo canal
  loopback `127.0.0.1:7668`, lista blanca de nombres.
- El protocolo del display acepta `scene` y `preset` (`field|sphere|orb|neural`). Se retiran las
  acciones `cinematic` (crown/atlas/dense) y `overlay` (hud/shell/ambient/guides/links/pulses/bloom):
  los parámetros visuales ya no se conmutan por capas, viven en presets.
- `field` = «Regiones vivas», sin sobrescrituras: se dibuja exactamente igual que antes.
- `sphere`, `orb`, `neural` solo cambian parámetros que este renderizador ya tiene.
- Los presets son archivos: `display/native/presets/<nombre>.json` (embebidos en `:/archie/presets`).
  Solo pueden fijar claves que existan en `defaults()`; una prueba de `archie-cli` exige que haya
  un archivo por cada preset que acepta la CLI y ninguno de más.
- Se retiró el botón «Antes» (`toggleCompare`, aspecto V2) del orbe nativo.

## No verificado (honesto)
- Sin Qt/CMake en la máquina de Xavi: los cambios de `display/native` (C++/QML, incluida la carga
  de presets JSON y el CMake) no se compilaron ni se vieron en pantalla. Los valores de los tres presets son una aproximación de diseño a
  revisar en el hardware real.
- No se portaron aún los nodos con halo/formas, los estilos de enlace ni el cerebro vivo
  (emisiones dentro de los enlaces) de Archie2: requieren shaders nuevos en `display/native/shaders`.

## Fuera de alcance
Áreas de Santiago y Penta (Sentry, Wanda, IRIS, Core, Policy).
