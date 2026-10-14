# Visor gráfico del escritorio

La esfera neuronal de **Archie Lumenfield 0449** como fondo del escritorio de ArchieOS. Muestra el
contenido del sistema operativo con los tres perfiles del programa (Esfera, Orbe y Campo),
navegable en 3D. **Solo para Arch/Linux.** No se carga el programa completo: solo la esfera.

Cómo funciona:

- **Contenido del sistema.** `/srv/archieos` contiene este repo, solo lo versionado
  (`git archive`, sin `.git` ni compilados).
- **Grafo guardado.** El análisis se hace **una sola vez** con el núcleo de Lumenfield
  (`grabar.sh`). Todas las respuestas que usa la esfera quedan en `/srv/visor/snap`: el grafo de
  cada perfil y presupuesto, los enlaces pensantes y la ficha y el contenido de cada nodo.
- **Visor.** `servidor.py` sirve el motor web de la esfera con ese grafo guardado. Al arrancar
  no usa el núcleo Rust ni escanea nada.
- **Archivos nuevos.** Un vigilante revisa `/srv/archieos` cada 3 s. Lo nuevo se agrega al grafo
  (enlazado a su carpeta), se guarda en `/srv/visor/nuevos.json` y la esfera se refresca sola.
  Lo existente no se recalcula.
- **Escritorio.** `display/shell/prototype/LiveOrb.qml` lo muestra detrás de las ventanas.
  - La cámara queda fija. Al pasar el ratón o hacer clic en un nodo aparece su ficha.
  - Ctrl+Espacio libera la cámara para navegar en 3D. Esc la vuelve a fijar.
  - Ajustes > Visor gráfico tiene el perfil, el brillo, la actividad y el giro.

## Puesta en marcha (una vez, como root)

```sh
LUMEN_WEB=/ruta/web/dist LUMEN_TEXTURES=/ruta/web/art/textures bash display/visor/preparar.sh
LUMEN_CORE=/ruta/al/binario/archie bash display/visor/grabar.sh
```

- `LUMEN_WEB`: la web compilada de Archie Lumenfield 0449. La que se probó tiene los enlaces más
  suaves (`SEG = 18` en `sphere-layer.ts`).
- `LUMEN_TEXTURES`: sus texturas (`web/art/textures`).
- `LUMEN_CORE`: el núcleo compilado para Linux, que solo hace falta para grabar.

Ni el motor web ni las texturas (unos 110 MB) se guardan en el repo: `web/` está en `.gitignore`.

## Uso

```sh
bash display/visor/escritorio.sh
```

Arranca el visor y la shell. En WSL usa la GPU por D3D12.

## Estado

- **Probado en WSL2 Arch con una GTX 1050 Ti.**
  - Grafo de 1.589 nodos, unos 29 FPS.
  - El vigilante agrega y quita archivos del grafo.
  - Ficha flotante al hacer clic en un nodo.
- **Sin probar.**
  - El refresco automático de la esfera en pantalla al llegar un archivo nuevo.
  - La navegación con Ctrl+Espacio.
  - El cambio de perfil desde Ajustes.
  - Un arranque en frío.
- **Limitación.** «Entrar carpeta» y la ficha solo tienen datos de lo grabado. Para un archivo
  nuevo se muestra su nombre, tipo, tamaño y fecha.
