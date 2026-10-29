# Instalar ArchieOS desde el lanzador

El lanzador (`ArchieOS-Launcher.ps1`, PowerShell 5.1 + WPF) gana una sección **Instalar ArchieOS**:
la tarjeta **ARCHIEOS** en la columna izquierda y la pestaña **Avanzado → Instalar**. Detecta el
equipo, analiza requisitos, descarga la imagen del release a la carpeta elegida, la verifica, la
importa en WSL 2, comprueba cada modelo (IRIS incluido) y después la inicia, la detiene, la
verifica o la repara. `ArchieOS-Launcher.ps1 -Page instalar` abre directamente esa pestaña.

Archivos (los tres juntos, en la misma carpeta):

| Archivo | Qué es |
|---|---|
| `ArchieOS-Launcher.ps1` | La ventana. Solo se le **añadieron** líneas (0 eliminadas): tarjeta, pestaña y su lógica. |
| `ArchieOS-Installer.ps1` | El motor. Cada paso largo corre como proceso propio, oculto: la ventana no se congela. |
| `archieos-manifest.json` | El release fijado: URL, revisión, tamaño, SHA-256 y formato de cada artefacto. |

## Auditoría (2026-10-28)

| Artefacto | Origen verificable | Versión | SHA-256 | Destino | Uso | Obligatorio |
|---|---|---|---|---|---|---|
| `ArchieOS-Bonsai2-9B-Xavi-61ad39f.wsl` (11 250 696 964 B) | `huggingface.co/PentaVM/ArchieOS-Bonsai2-9B-WSL` @ `eea2dfd2…`, publicado en el release borrador de `PentaVM/ArchieOS-Nvidia` (privado: **401 sin token**) | código `61ad39f` | `3634e03a50e9f36ecc2f0b29208c1dc6dcfc3dfa9d2879867b0b12e9579e5dab` | `<carpeta>\downloads` → `wsl --import` en `<carpeta>\wsl\ArchieOS-Bonsai-Xavi` | La imagen completa: Arch Linux, Core, escritorio, Bonsai2 9B, ASR, Piper, IRIS | sí |
| `iris-beta21-step132000.pt` (1 414 806 323 B) | Dentro de la imagen; además en `PentaVM/archie-iris` release `iris-beta2.1-2026-09-28` (privado) | IRIS Beta 2.1, step 132000 | `c6cadb02f5a5bffc4ea3c3348d32b475ef7531f643675c06c65451037e4f3b97` | `/var/lib/archie/models/iris/` en la distribución | Ojos y manos de Archie; se comprueba con `archie-offline --check` | sí (no se descarga aparte: sería un duplicado) |
| `ArchieOS-Operator.wsl` + `SHA256SUMS` + `install-archie-operator.ps1` | **No existe**: `PentaVM/ArchieOS-Nvidia` no tiene ningún release con assets | — | — | — | Lo que construye `scripts/wsl/build-operator.sh` | **bloqueo** |
| OVA de VirtualBox | **No existe** | — | — | — | — | no |
| Paquete nativo Linux | **No existe** (solo ISOs archiso de `PentaVM/ArchieOS` v0.1.0-alpha) | — | — | — | — | no |

Lo que la auditoría comprobó en el código, no en los nombres:

- **Formato**: `scripts/wsl/package-offline.sh` crea la imagen con `tar … | gzip -n -1` y `gzip -t`.
  Es un rootfs TAR comprimido con gzip, que `wsl --import` acepta. El instalador no se fía de la
  extensión `.wsl`: lee los bytes `1f 8b`, descomprime la primera cabecera y busca `ustar`.
- **Nombre de la distribución**: `ArchieOS-Bonsai-Xavi` (el empaquetador exige
  `defaultName=ArchieOS-Bonsai-Xavi` en `/etc/wsl-distribution.conf`). Se puede cambiar en la pestaña.
- **Arranque real**: `/usr/bin/archie-offline` → `offline-session.py`. `--check` verifica el
  SHA-256 de Bonsai, ASR, Piper ES/EN e IRIS; `--background`, como usuario `archie` (nunca root),
  levanta PostgreSQL, Bonsai (`:8081`), `archied`, `archie-displayd` (`:7666`), el escritorio y la voz.
  Tiene su propio candado y nunca reemplaza un puerto ajeno.
- **Salud**: lo que `offline-session.py` espera: `archie --root …/node status`,
  `GET :7666/api/orb/lumen` con `root_id`, `GET :8081/health` = `ok`, y los procesos que registró
  (pid + instante de inicio).
- **Parada**: la imagen no trae comando de parada. El lanzador hace `pg_ctl stop -m fast` y luego
  `wsl --terminate <distribución>`: solo esa; WSL y las demás distribuciones siguen.
- **Primer inicio**: con `wsl --import` no corre la OOBE de la imagen, así que el instalador repite
  sus pasos: `install -d /home/archie/.local/state`, `/usr/lib/archie/init-pacman-keyring` (root) y
  `archie-offline --check`.
- **`archie-node` no existe** en ninguna rama de `PentaVM/ArchieOS-Nvidia` ni en la imagen: la
  tarjeta NODO original lo invoca, pero la nueva sección no depende de él (y no se tocó).
- **Dependencias del host**: el lanzador es PowerShell/WPF → necesita .NET Framework 4.5+. No hay
  binarios nativos de Windows: **no** se exige VC++ Redistributable, WebView2, Vulkan ni DirectX. La
  GPU es opcional: la imagen arranca en CPU por defecto.

## Arquitectura

| Responsabilidad del encargo | Dónde |
|---|---|
| PlatformProbe | `Get-HostProbe`, `Get-WslVersionInfo`, `Get-WslRegistration`, `Get-FolderVolume` |
| DependencyManager | `Get-DependencyPlan`, `Invoke-DependencyFix` (solo `wsl --install --no-distribution` y `wsl --update`, elevados y con confirmación) |
| ReleaseResolver / ManifestVerifier | `Read-ReleaseManifest` (versión fijada, HTTPS, sin `latest`, SHA-256 obligatorio, formato conocido) |
| DownloadManager | `Invoke-ArtifactDownload`, `Open-ArtifactResponse`, `Test-ArtifactFormat` |
| InstallOrchestrator | `Invoke-Install`, `Import-Distro`, `Initialize-Distro`, `Invoke-Repair`, `Invoke-Adopt` |
| RuntimeController | `Start-ArchieRuntime`, `Stop-ArchieRuntime`, `Test-ArchieHealth`, `Test-IrisLoad` |
| InstallStateStore | `Read-InstallState`, `Save-InstallState` (escritura atómica), `Set-InstallState` (transiciones validadas) |

La ventana llama al motor con el mismo patrón que ya usaba para `wsl.exe` (`$script:jobs` + el
temporizador `$pump`): cada paso es `powershell.exe -File ArchieOS-Installer.ps1 <verbo> …`, sus
líneas van al registro de la ventana y su resultado (una línea JSON) al callback.

Línea de comandos (lo mismo que hace la ventana):

```
ArchieOS-Installer.ps1 probe | plan [-Root DIR] [-Distro NOMBRE]
ArchieOS-Installer.ps1 install -Root DIR [-Distro NOMBRE]
ArchieOS-Installer.ps1 start | stop | health | status -Root DIR
ArchieOS-Installer.ps1 verify -Root DIR [-Deep]
ArchieOS-Installer.ps1 repair -Root DIR [-AllowUnregisterBroken]
ArchieOS-Installer.ps1 adopt -Root DIR -Distro NOMBRE
ArchieOS-Installer.ps1 fix-dependency -Id wsl|wsl-update [-Root DIR]
```

## Carpeta de instalación

```
<carpeta>\downloads\ArchieOS-Bonsai2-9B-Xavi-61ad39f.wsl   (.part mientras baja; caché reutilizable)
<carpeta>\wsl\<distribución>\ext4.vhdx                      el disco virtual de WSL
<carpeta>\state\install.json                                estado persistente
<carpeta>\state\progress.json                               progreso del paso en curso
<carpeta>\logs\installer.log                                registro (sin tokens)
```

Se sugiere por defecto la unidad fija NTFS con más espacio. Las rutas con espacios, tildes y otras
unidades llegan enteras: cada argumento se pasa con las reglas de `CommandLineToArgvW`, sin
`cmd.exe` ni shell de por medio. Se rechazan unidades de red, FAT32/exFAT y carpetas comprimidas
por NTFS (WSL 2 no monta ahí su disco). Espacio pedido: lo que falte de la descarga + 40 GB
(estimación del release para la distribución).

## Estados

`not_installed`, `checking`, `needs_dependencies`, `downloading`, `verifying`, `installing`,
`needs_reboot`, `installed`, `starting`, `running`, `stopped`, `error`, `repairable`, guardados en
`state\install.json` con la distribución, su ubicación, el release, los artefactos con su SHA-256 y
los puntos de control (`downloaded`, `imported`, `initialized`). `Set-InstallState` rechaza los
saltos que la tabla `$Transitions` no permite.

- Cerrar el lanzador durante un paso no lo corta: el proceso sigue y, al reabrir, la ventana lo
  reconoce por su pid. Si el paso murió, la tarjeta dice **Instalación interrumpida** y
  **Continuar instalación** retoma: la descarga se reanuda con `Range` y una importación ya hecha
  no se repite.
- Tras `wsl --install`, el estado queda en `needs_reboot`; al abrir el lanzador después de
  reiniciar (arranque de Windows posterior a la petición), vuelve solo a `not_installed`.
- **En ejecución** solo aparece cuando la salud responde, no porque se lanzó un proceso.

## Descarga

- HTTPS, revisión fijada en el manifiesto; nunca `latest`.
- Repositorio privado: el token se guarda **cifrado con DPAPI** para el usuario de Windows en
  `%APPDATA%\ArchieOS\credentials\<proveedor>.dpapi`, desde la pestaña (campo de contraseña) o con:
  `. .\ArchieOS-Installer.ps1; Save-ProviderToken huggingface (Read-Host -AsSecureString)`.
  Se envía solo al host al que pertenece (`huggingface.co`, `api.github.com`): las redirecciones se
  siguen a mano y el almacenamiento al que redirigen no lo recibe. Los registros enmascaran
  `hf_…`, `ghp_…` y cabeceras `Authorization`.
- Streaming a disco por bloques de 1 MB, con el SHA-256 calculado mientras llega (no se carga la
  imagen en memoria). Se renombra de `.part` al nombre final solo si tamaño y SHA-256 coinciden.
- Tiempo de espera de 30 s por conexión y 60 s por lectura; 4 reintentos con espera creciente;
  reanudación con `Range`; si el servidor no reanuda, empieza de cero.
- 401/403 → `[auth]`, 404 → `[not_found]`, SHA-256 distinto → `[checksum]` (se borra lo bajado),
  cancelación → `[cancelled]` (se conserva el `.part`).
- Sin firma criptográfica: el release no publica ninguna (`provenance.signature: null`).

## Seguridad de lo que ya existe

- Una distribución con el mismo nombre **nunca** se sobrescribe ni se desregistra sola. Si es de
  otra instalación: **Reutilizar** (solo tras pasar su propio `archie-offline --check`) o instalar
  con otro nombre.
- Una distribución registrada **sin su disco** (como `ArchieOS-Operator` en el equipo de la
  auditoría: `X:\WSL\ArchieOS-Operator\ext4.vhdx` no existe) se marca reparable. Reparar la quita
  con `wsl --unregister` solo si confirmas, y solo tras comprobar otra vez que el disco no está.
- Un `ext4.vhdx` suelto en la carpeta de destino no se toca.
- Si `wsl --import` falla, solo se borra la carpeta vacía que creó el propio paso.
- En Linux el motor solo informa: no hay paquete nativo y no escribe en discos, particiones, GRUB
  ni EFI.

## Solución de errores

| Mensaje | Qué hacer |
|---|---|
| «hace falta un token…» / `[auth]` | Guarda un token de una cuenta con acceso a `PentaVM/ArchieOS-Bonsai2-9B-WSL`. |
| `[not_found]` | El archivo ya no está en esa revisión: hay que actualizar `archieos-manifest.json`. |
| `[checksum]` | Se borró lo descargado; **Continuar instalación** lo baja otra vez. |
| «WSL no está instalado» | **Instalar requisitos** → `wsl --install --no-distribution` (pide administrador y reiniciar). |
| «WSL x.y; la imagen pide 2.4.4» | **Instalar requisitos** → `wsl --update`. |
| «Virtualización…» | Actívala en la BIOS/UEFI. |
| «registrada sin su disco» | **Verificar/Reparar**, y confirma quitar el registro roto. |
| «loopback port … belongs to another service» | Otra distribución usa 8081/7666/7668: detenla y vuelve a iniciar. |
| `[health]` | Arrancó pero algo no responde: **Registros** abre `<carpeta>\logs` y `~/.local/state/archie-offline` de la distribución. |

## No probado (a petición)

Por indicación expresa no se descargó, no se instaló nada y no se ejecutaron pruebas. Lo único que
se ejecutó fue `probe`/`plan` (solo lectura) durante la auditoría, sobre este equipo: Windows 11 Pro
26300, WSL 3.0.3.0 con WSLg, GTX 1050 Ti, sin administrador; detectó la distribución rota
`ArchieOS-Operator` y avisó de la memoria de WSL (8,5 GB frente a 8,6 GB). La revisión final fue
estática: análisis sintáctico de ambos scripts (0 errores), XAML bien formado, cada control
referenciado existe y el manifiesto es JSON válido.

Sin verificar, por tanto: la ventana abierta con la nueva sección, la descarga real (necesita el
token), `wsl --import`, el arranque y la salud, la parada, el segundo inicio, la reanudación, la
cancelación y los casos 401/403/404/checksum.

## Pendiente

1. **Publicar la imagen Operator** (o hacer público el archivo de Hugging Face) con su SHA-256 en
   un release, idealmente desde CI, y una firma si se quiere verificar autoría.
2. Un token de Hugging Face con acceso para quien instale mientras la imagen sea privada.
3. Decidir dónde vive el lanzador (no está en `PentaVM/ArchieOS-Nvidia`): sin repositorio no hay PR.
4. **Actualizar**: no se ofrece porque no existe un canal de releases que consultar; hoy se hace
   cambiando el manifiesto.
