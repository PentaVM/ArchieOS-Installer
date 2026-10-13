# Verificación previa al merge de Xavi — 2026-10-13

Sin modificar las ramas del autor. Fuentes exportadas con `git archive` a directorios
temporales independientes. Estos archivos son herramientas de auditoría, no una GUI
nueva ni componentes de producción.

| Fuente | SHA probado |
|---|---|
| PR40, Orbe | `e46034161b9667a1ca2b696d61600b024b3610c4` |
| PR41, shell | `9f98235ab9e220ddb886e61bfa2df235a751ec3e` |
| main, comparación de integración | `8e0cf223e271f26989caeb43be5fc8809bdac131` |

## Resultados independientes

- PR40: CMake/Ninja, GCC 16.2.1, Qt 6.11.2 y zlib 1.3.2 en ArchieOS-Builder.
  `display/native` compiló completo, incluidos C++, registros QML, cachés QML y
  recursos Qt. Pruebas visuales **19/19**, OSM **2/2**.
- `NativePresetReview.cpp`: **82 comprobaciones, cero fallos**, enlazado contra los
  objetos y recursos del display recién compilado. Comprueba los cuatro JSON
  embebidos, selección de presets, aplicación de valores, reset, restauración de
  defaults de `field` y fallback ante nombre desconocido. No levanta observadores.
- Binario nativo en WSLg/Wayland: arranque y `--render-check` correctos, PNG de
  836791 bytes, salida 0. Cuatro presets y tres escenas respondieron `ok` en 7668.
  JSON malformado, preset desconocido, antiguos cinematic/overlay y petición de
  más de 4096 bytes fueron rechazados. Que los antiguos comandos sean rechazados
  confirma una ruptura de compatibilidad; no prueba preservación funcional.
- El log identifica **Mesa llvmpipe**, OpenGL 4.6, y errores previos de Zink/EGL.
  Fue render por software, **no validación de aceleración NVIDIA**, FPS ni VRAM.
  No había observador en 7666: el PNG muestra esfera/partículas y ausencia de
  conexión, **no nodos reales ni semántica de presets con datos vivos**.
- PR41: Qt 6.11.2 stock, Linux y Windows/MSYS2. Se observaron **21/21** resultados
  BridgeModel y **79/79** SurfacesTest. En Windows ambos procesos terminaron con
  salida 0; el harness combinado `ShellReview.qml` también pasó ambas suites.
  La prueba Linux reportó esas cifras, pero la ejecución tuvo timeouts/problemas
  temporales de conexión WSL: no usar su estado de salida como prueba de limpieza.
- Ventana Qt Quick offscreen/software en Windows: escritorio a 1920×1080,
  recorrido de 14 overlays, PNG obtenido y salida 0 sin avisos QML en ese recorrido
  de estado normal. Datos de diseño, sin servicios conectados.
- La misma ventana de muestra recorrió los 14 overlays y guardó PNG bajo
  WSLg/Wayland, salida 0 y sin avisos QML del recorrido normal. OpenGL con llvmpipe
  y errores previos de Zink/EGL: tampoco acredita aceleración NVIDIA.

## Hallazgos que impiden considerar terminada la integración

1. **PR40: funciones retiradas.** Desaparecen herramientas de voz `orb_cinematic`
   y `orb_overlay`, protocolo cinematic/overlay, perfiles crown/atlas/dense y
   `toggleCompare`/Antes. Resolver preservación/migración antes del merge.
   `CAPABILITY_REGISTRY.csv` y `CAPABILITY_SYMBOL_INDEX.csv` conservan referencias
   a herramientas/símbolos retirados. Esto afecta el inventario para portar la GUI.
2. **PR40: CI Rust rojo por formato.** Reproducido con rustfmt sobre copia exacta
   de `visual.rs`: líneas 138 y 147. El CI se detiene antes de check/test/clippy.
3. **PR41: aviso QML reproducible.** `WindowSwitcher.qml:35` evalúa
   `color: Theme.dk[info.key]`. La tarjeta de app desconocida en estado degraded
   tiene `key: ""`; el mapa no contiene esa clave. `visible: false` no evita
   evaluar el binding y Qt informa `Unable to assign [undefined] to QColor`.
   Se reprodujo en Windows y Linux; las 79 assertions no cuentan los avisos.
   La guarda de iconos del último commit no protege este binding de color.
4. **Despliegue pendiente.** La shell sigue siendo prototipo Qt Quick: Quickshell,
   instalación dentro del WSL distribuible, conexión principal por voz, datos
   reales, Wayland/NVIDIA, blur, accesibilidad y rendimiento no quedan acreditados
   por estas pruebas. `native-contracts` no compila `display/native` ni esta shell.

`git merge-tree --write-tree main PR40` y `main PR41` no mostraron conflictos.
Eso comprueba combinación textual, no despliegue ni preservación de capacidades.
No se fusionó ninguna rama ni se modificó la imagen/release de Sheriff.

## Reproducir el build C++ y los presets

En un Linux con Qt Quick/Controls/Quick3D/Network/Concurrent 6.6+, GCC, CMake,
Ninja, pkg-config y zlib, exportar el SHA de PR40 a `SNAPSHOT`:

```sh
git archive e46034161b9667a1ca2b696d61600b024b3610c4 display/native display/visual display/osm | tar -x -C "$SNAPSHOT"
cmake -S "$SNAPSHOT/display/native" -B "$SNAPSHOT/build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DARCHIE_VISUAL_BUILD_TESTS=ON -DARCHIE_OSM_BUILD_TESTS=ON
cmake --build "$SNAPSHOT/build" --parallel 2
ctest --test-dir "$SNAPSHOT/build/archie-visual" --output-on-failure
ctest --test-dir "$SNAPSHOT/build/archie-osm" --output-on-failure
bash _dev/verification/xavi/run-native-presets.sh "$SNAPSHOT" "$PWD/_dev/verification/xavi"
```

El script de presets reutiliza los objetos del build y sus recursos, excluye
`main.cpp.o` y no altera las fuentes exportadas. La disposición de objetos es la
de este CMake/Ninja; no se afirma soporte del harness en MSVC.

## Reproducir las suites QML

Exportar `display/shell` del SHA de PR41 en otro directorio. Copiar
`_dev/verification/xavi/ShellReview.qml` manteniendo su ruta relativa al directorio
exportado. En Linux:

```sh
LC_ALL=C.UTF-8 QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software \
  timeout 20 /usr/lib/qt6/bin/qml _dev/verification/xavi/ShellReview.qml >qtests.log 2>&1
```

Exigir ambos `REVIEW_RESULT` con 21 y 79 passes, cero failures, sin `loadFailed`,
salida sin timeout y revisar también los avisos del log. **Salida 0 no basta.**
En Windows usar `Start-Process -WindowStyle Hidden` con stdout/stderr redirigidos,
Qt en PATH y límite de 20 segundos, para recoger la salida del ejecutable GUI.
Qt 6.6 mínimo, Qt 6.8.3/MSVC del autor y su Qt 6.12 no fueron reproducidos aquí.

`ShellWindowReview.qml`, con la misma disposición relativa, recorre los 14
overlays en estado normal y escribe `review-desktop.png`. Ejecutarlo con límite
de 25 segundos y comprobar `REVIEW_RENDER true`, la imagen y el log completo.
Para repetir la prueba Wayland quitar offscreen/software y definir
`QT_QPA_PLATFORM=wayland QSG_RHI_BACKEND=opengl QSG_INFO=1`. Examinar el renderer
del log; un PNG correcto con llvmpipe no es una prueba de la GPU.
