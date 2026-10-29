# ArchieOS texts. English is the default and the key: T 'text' {args} returns the text in the
# language chosen in the launcher (Advanced > Startup), or the English when a translation is
# missing. {0}, {1} are filled by the arguments; \n is a new line. Add a language: add a table
# below and its name to $script:Languages.
#
# One line per text. The English key is the text written in the scripts, so a missing line
# only means that text shows in English.
if (-not $script:I18nLoaded) {
$script:I18nLoaded = $true
if (-not $script:UiLang) { $script:UiLang = 'en' }
$script:Languages = [ordered]@{ en = 'English'; es = 'Español' }
$script:I18n = @{}
$script:I18n['es'] = @{
    'no response in {0} s' = 'sin respuesta en {0} s'
    'Invalid distribution name: {0}' = 'Nombre de distribución no válido: {0}'
    'The release manifest has no signature (archieos-manifest.json.sig).' = 'The release manifest has no signature (archieos-manifest.json.sig).'
    'The signature file is not valid JSON.' = 'The signature file is not valid JSON.'
    'Unknown signature algorithm: {0}' = 'Unknown signature algorithm: {0}'
    'The manifest is signed by an unknown key ({0}). Download a newer installer.' = 'The manifest is signed by an unknown key ({0}). Download a newer installer.'
    'The release manifest signature does not match. It was changed or is not from ArchieOS; nothing was installed.' = 'The release manifest signature does not match. It was changed or is not from ArchieOS; nothing was installed.'
    'WARNING: signature not checked (-AllowUnsigned).' = 'WARNING: signature not checked (-AllowUnsigned).'
    'Manifest signed by key {0}: valid.' = 'Manifest signed by key {0}: valid.'
    'Cannot find the manifest {0}' = 'No encuentro el manifiesto {0}'
    'The manifest is not valid JSON: {0}' = 'El manifiesto no es JSON válido: {0}'
    'Unsupported manifest version: {0}' = 'Versión de manifiesto no soportada: {0}'
    'The manifest must pin a version, not "latest".' = 'El manifiesto debe fijar una versión, no «latest».'
    'The manifest has no windows-x86_64-wsl2 target.' = 'El manifiesto no tiene destino windows-x86_64-wsl2.'
    'Unknown startup mechanism: {0}' = 'Mecanismo de arranque desconocido: {0}'
    'Every artifact needs a unique id.' = 'Cada artefacto necesita un id único.'
    '{0}: no published SHA-256; nothing is installed that cannot be checked.' = '{0}: sin SHA-256 publicado; no se instala lo que no se puede comprobar.'
    '{0}: the size is missing.' = '{0}: falta el tamaño.'
    '{0}: the source must be an HTTPS URL.' = '{0}: la fuente debe ser una URL HTTPS.'
    '{0}: the source cannot point to "latest".' = '{0}: la fuente no puede apuntar a «latest».'
    '{0}: invalid file name.' = '{0}: nombre de archivo no válido.'
    '{0}: unknown authentication {1}.' = '{0}: autenticación desconocida {1}.'
    '{0}: unsupported format {1}.' = '{0}: formato no soportado {1}.'
    '{0}: it has neither a source nor an image that contains it.' = '{0}: ni fuente ni imagen que lo contenga.'
    '{0}: {1} is not in the manifest.' = '{0}: {1} no está en el manifiesto.'
    'The target needs exactly one required image to download.' = 'El destino necesita exactamente una imagen obligatoria que descargar.'
    'Invalid state transition: {0} → {1}' = 'Paso de estado no válido: {0} → {1}'
    'System' = 'Sistema'
    '{0}: this launcher installs ArchieOS on Windows with WSL 2. There is no published package for Linux: only archiso ISOs, which are boot media and are not written to disks from here.' = '{0}: este lanzador instala ArchieOS en Windows con WSL 2. Para Linux no hay paquete publicado: solo ISOs archiso, que son medios de arranque y no se escriben en discos desde aquí.'
    '{0} (build {1})' = '{0} (compilación {1})'
    '{0} (build {1}); WSLg comes with the Microsoft Store WSL' = '{0} (compilación {1}); WSLg llega con el WSL de Microsoft Store'
    'Build {0}: WSLg needs Windows 11, or Windows 10 21H2 (19044) or later.' = 'Compilación {0}: WSLg necesita Windows 11, o Windows 10 21H2 (19044) o posterior.'
    '{0}: the ArchieOS image is x86_64.' = '{0}: la imagen de ArchieOS es x86_64.'
    'Virtualization' = 'Virtualización'
    'Windows hypervisor active' = 'hipervisor de Windows activo'
    'enabled in the firmware' = 'activada en el firmware'
    'Virtualization is not enabled on the CPU.' = 'La CPU no tiene la virtualización activada.'
    'Enable it (Intel VT-x or AMD-V/SVM) in the BIOS/UEFI; it cannot be done from Windows.' = 'Actívala (Intel VT-x o AMD-V/SVM) en la BIOS/UEFI; no se puede hacer desde Windows.'
    'WSL was installed: Windows has to restart before going on.' = 'Se instaló WSL: Windows tiene que reiniciarse antes de seguir.'
    'Restart Windows and open the launcher again.' = 'Reinicia Windows y vuelve a abrir el lanzador.'
    'WSL is not installed (or it is the old WSL built into Windows).' = 'WSL no está instalado (o es el WSL antiguo integrado en Windows).'
    'wsl --install --no-distribution: installs WSL and the Virtual Machine Platform, with no distribution. It asks for administrator permission and a restart.' = 'wsl --install --no-distribution: instala WSL y la Plataforma de máquina virtual, sin ninguna distribución. Pide permiso de administrador y reiniciar.'
    'WSL {0}; the image needs {1} or later.' = 'WSL {0}; la imagen pide {1} o posterior.'
    'wsl --update: updates WSL from Microsoft. It asks for administrator permission.' = 'wsl --update: actualiza WSL desde Microsoft. Pide permiso de administrador.'
    'WSLg (display and sound)' = 'WSLg (pantalla y sonido)'
    'included in this WSL' = 'incluido en este WSL'
    'This WSL does not report WSLg; the desktop and the voice need it.' = 'Este WSL no informa WSLg; el escritorio y la voz lo necesitan.'
    'Pending restart' = 'Reinicio pendiente'
    'Windows has a pending restart. If WSL fails to import, restart before trying again.' = 'Windows tiene un reinicio pendiente. Si WSL falla al importar, reinicia antes de reintentar.'
    'Memory for WSL' = 'Memoria para WSL'
    'WSL will have {0}; the release asks for {1}. It may be slow or fail to load the model.' = 'WSL tendrá {0}; el release pide {1}. Puede ir lento o no cargar el modelo.'
    'If needed, set memory=8GB in the [wsl2] section of %USERPROFILE%\.wslconfig.' = 'Si hace falta, pon memory=8GB en la sección [wsl2] de %USERPROFILE%\.wslconfig.'
    '{0}, driver {1}. The image starts on CPU; the GPU is not used by default.' = '{0}, driver {1}. La imagen arranca en CPU; la GPU no se usa por defecto.'
    'There is no NVIDIA GPU: ArchieOS runs on CPU, its default mode (slow voice answers).' = 'No hay GPU NVIDIA: ArchieOS corre en CPU, que es su modo por defecto (respuestas de voz lentas).'
    '.NET Framework (launcher window)' = '.NET Framework (ventana del lanzador)'
    '4.x, version {0}' = '4.x, versión {0}'
    'The launcher window needs .NET Framework 4.5 or later.' = 'La ventana del lanzador necesita .NET Framework 4.5 o posterior.'
    'Install it from Windows Update.' = 'Instálalo desde Windows Update.'
    'Folder' = 'Carpeta'
    '{0} is on a network drive: WSL does not keep its disk there.' = '{0} está en una unidad de red: WSL no guarda su disco ahí.'
    'Choose a folder on a local disk.' = 'Elige una carpeta en un disco local.'
    'Drive {0} is not available.' = 'La unidad {0} no está disponible.'
    'Connect the disk or choose another folder.' = 'Conecta el disco o elige otra carpeta.'
    '{0} is {1}: the WSL virtual disk needs NTFS or ReFS (FAT32 and exFAT will not do).' = '{0} es {1}: el disco virtual de WSL necesita NTFS o ReFS (FAT32 y exFAT no sirven).'
    'Choose another drive.' = 'Elige otra unidad.'
    '{0} is compressed by NTFS: WSL 2 does not mount a compressed virtual disk.' = '{0} está comprimida por NTFS: WSL 2 no monta un disco virtual comprimido.'
    'Remove the compression from the folder or choose another.' = 'Quita la compresión de la carpeta o elige otra.'
    'Disk space' = 'Espacio en disco'
    '{0} are needed (download {1} + distribution {2}) and {3} has {4} left.' = 'Hacen falta {0} (descarga {1} + distribución {2}) y en {3} quedan {4}.'
    'Free up space or choose another drive.' = 'Libera espacio o elige otra unidad.'
    '{0} free on {1}; {2} needed' = '{0} libres en {1}; hacen falta {2}'
    'Distribution {0}' = 'Distribución {0}'
    'free: it will be created on install' = 'libre: se creará al instalar'
    'It is registered in WSL, but its disk {0} does not exist.' = 'Está registrada en WSL, pero su disco {0} no existe.'
    'Repair: remove that broken registration (it has no data to lose) and import again; or use another name.' = 'Reparar: quitar ese registro roto (no tiene datos que perder) e importar de nuevo; o usar otro nombre.'
    'installed by this launcher in {0}' = 'instalada por este lanzador en {0}'
    'A distribution {0} already exists in {1} that this folder did not install.' = 'Ya existe una distribución {0} en {1} que no instaló esta carpeta.'
    'Reuse it (it is checked first) or install under another name. It is not overwritten or deleted.' = 'Reutilizarla (se comprueba antes) o instalar con otro nombre. No se sobrescribe ni se borra.'
    'There is no automatic fix for {0}' = 'No hay corrección automática para {0}'
    'Administrator permission was not given: nothing was changed.' = 'No se dio el permiso de administrador: no se cambió nada.'
    'wsl.exe {0} exited with code {1}.' = 'wsl.exe {0} terminó con código {1}.'
    'The download needs access to {0}: save a token in Install → Access.' = 'La descarga necesita acceso a {0}: guarda un token en Instalar → Acceso.'
    'Redirect to a URL that is not HTTPS: {0}' = 'Redirección a una URL que no es HTTPS: {0}'
    '{0} answered {1}: the token has no access to this file (or it expired).' = '{0} respondió {1}: el token no tiene acceso a este archivo (o caducó).'
    '{0} answered 404: the release file does not exist at that address.' = '{0} respondió 404: el archivo del release no existe en esa dirección.'
    '{0} answered {1}.' = '{0} respondió {1}.'
    '{0} is already downloaded: checking its SHA-256' = '{0} ya está descargado: comprobando su SHA-256'
    'It does not match the release: downloading again.' = 'No coincide con el release: se descarga de nuevo.'
    'Resuming the download at {0} of {1}' = 'Continuando la descarga en {0} de {1}'
    '{0} more free are needed on {1} for the download.' = 'Faltan {0} libres en {1} para la descarga.'
    'Download cancelled; what was downloaded is kept and Retry continues.' = 'Descarga cancelada; lo descargado se conserva y Reintentar continúa.'
    'The server does not resume downloads: starting from zero.' = 'El servidor no reanuda descargas: empieza de cero.'
    'The server sends more bytes than the release has.' = 'El servidor envía más bytes que los del release.'
    'The download was cut {0} times: {1}. Retry continues where it left off.' = 'La descarga se cortó {0} veces: {1}. Reintentar continúa donde quedó.'
    'The connection dropped ({0}); retry {1} of {2}' = 'Se cortó la conexión ({0}); reintento {1} de {2}'
    '{0} does not match the release (SHA-256 {1}). What was downloaded was deleted; Retry downloads it again.' = '{0} no coincide con el release (SHA-256 {1}). Se borró lo descargado; Reintentar lo baja de nuevo.'
    'format {0} not supported' = 'formato {0} no soportado'
    'not a gzip file' = 'no es un archivo gzip'
    'the gzip does not contain a complete tar' = 'el gzip no contiene un tar completo'
    'the gzip does not contain a tar file' = 'el gzip no contiene un archivo tar'
    'the distribution is not running' = 'la distribución no está en marcha'
    'could not read the state: {0}' = 'no se pudo leer el estado: {0}'
    'ArchieOS is not installed in this folder.' = 'ArchieOS no está instalado en esta carpeta.'
    'The distribution {0} is no longer registered in WSL or its disk is missing.' = 'La distribución {0} ya no está registrada en WSL o le falta su disco.'
    'The distribution {0} is gone (or its disk is missing): Verify/Repair re-imports it.' = 'La distribución {0} ya no está (o le falta su disco): Verificar/Reparar la reimporta.'
    'archie-offline --background in {0} (the first time it prepares the database)' = 'archie-offline --background en {0} (la primera vez prepara la base de datos)'
    'It did not start: {0}' = 'No arrancó: {0}'
    'ArchieOS did not start: {0}' = 'ArchieOS no arrancó: {0}'
    'It started, but {0}' = 'Arrancó, pero {0}'
    'ArchieOS started, but {0}. Logs: \\wsl.localhost\{1}{2}' = 'ArchieOS arrancó, pero {0}. Registros: \\wsl.localhost\{1}{2}'
    'Could not prepare /home/archie/.local/state: {0}' = 'No se pudo preparar /home/archie/.local/state: {0}'
    'Generating the package keys for this installation' = 'Generando las claves de paquetes de esta instalación'
    'init-pacman-keyring failed: {0}' = 'init-pacman-keyring falló: {0}'
    'archie-offline --check: checking the SHA-256 of each model inside the image (IRIS included)' = 'archie-offline --check: comprobando el SHA-256 de cada modelo dentro de la imagen (IRIS incluido)'
    'Integrity OK: {0} models, source {1}' = 'Integridad correcta: {0} modelos, código {1}'
    'Loading IRIS on CPU to check that its weights can be read (up to 10 minutes)' = 'Cargando IRIS en CPU para comprobar que sus pesos se leen (hasta 10 minutos)'
    '{0} is already imported at {1}: continuing' = '{0} ya está importada en {1}: se continúa'
    '{0} is registered without its disk ({1}). Verify/Repair can remove that broken registration if you confirm.' = '{0} está registrada sin su disco ({1}). Verificar/Reparar puede quitar ese registro roto si lo confirmas.'
    'A distribution {0} already exists in {1}. It is not overwritten: reuse it or install under another name.' = 'Ya existe una distribución {0} en {1}. No se sobrescribe: reutilízala o instala con otro nombre.'
    'There is already an ext4.vhdx disk in {0} that WSL has not registered. It is not touched: move it or choose another folder.' = 'En {0} ya hay un disco ext4.vhdx que WSL no tiene registrado. No se toca: muévelo o elige otra carpeta.'
    'wsl --import {0} (a few minutes; the virtual disk goes to {1})' = 'wsl --import {0} (unos minutos; el disco virtual queda en {1})'
    'wsl --import failed: {0}' = 'wsl --import falló: {0}'
    'ArchieOS is already installed in {0} ({1}).' = 'ArchieOS ya está instalado en {0} ({1}).'
    'missing: {0}: {1}' = 'falta: {0}: {1}'
    '{0}: {1}; wsl --import does not accept it.' = '{0}: {1}; wsl --import no lo acepta.'
    '{0}: gzip-compressed tar, importable with wsl --import' = '{0}: tar comprimido con gzip, importable con wsl --import'
    'There is no installation to repair in this folder.' = 'No hay ninguna instalación que reparar en esta carpeta.'
    '{0} is registered without its disk ({1}). Repair needs your confirmation to remove that registration.' = '{0} está registrada sin su disco ({1}). Reparar necesita tu confirmación para quitar ese registro.'
    'The disk appeared: nothing is removed.' = 'El disco apareció: no se quita nada.'
    'wsl --unregister {0} (registration without a disk, confirmed)' = 'wsl --unregister {0} (registro sin disco, confirmado)'
    '{0} exists in {1}, which does not belong to this installation. It is not touched.' = '{0} existe en {1}, que no es la de esta instalación. No se toca.'
    'There is no usable distribution {0}.' = 'No hay una distribución {0} utilizable.'
    'ArchieOS-HostModel.ps1 is missing next to the launcher' = 'falta ArchieOS-HostModel.ps1 junto al lanzador'
    'Reading…' = 'Leyendo…'
    'Advanced' = 'Avanzado'
    'Start now' = 'Iniciar ahora'
    'Do not start this time' = 'No iniciar esta vez'
    'Do not start with Windows' = 'No iniciar con Windows'
    'Start ArchieOS' = 'Iniciar ArchieOS'
    'Install ArchieOS' = 'Instalar ArchieOS'
    'Stop' = 'Detener'
    'Verify/Repair' = 'Verificar/Reparar'
    'Cancel' = 'Cancelar'
    'Logs' = 'Registros'
    'NODE' = 'NODO'
    'Start' = 'Iniciar'
    'Open GUI' = 'Abrir GUI'
    'Talk to Archie' = 'Hablar con Archie'
    'Start ArchieOS when I sign in to Windows' = 'Iniciar ArchieOS al entrar a Windows'
    'Free up disk space' = 'Liberar espacio en disco'
    'SERVICES' = 'SERVICIOS'
    'SETTINGS' = 'CONFIGURACIÓN'
    'Advanced shows the local model wizard, the start with Windows, the target (WSL or VirtualBox) and the log.' = 'Avanzado muestra el asistente del modelo local, el inicio con Windows, el destino (WSL o VirtualBox) y el registro.'
    'Local model' = 'Modelo local'
    'Also show the ones that do not fit on this computer' = 'Mostrar también los que no caben en este equipo'
    'Models folder:' = 'Carpeta de modelos:'
    'Change' = 'Cambiar'
    'Install' = 'Instalar'
    '1 · SYSTEM' = '1 · SISTEMA'
    '2 · MODE' = '2 · MODO'
    '3 · FOLDER' = '3 · CARPETA'
    'Choose…' = 'Elegir…'
    'WSL distribution:' = 'Distribución de WSL:'
    '4 · DOWNLOAD ACCESS' = '4 · ACCESO A LA DESCARGA'
    'Forget' = 'Olvidar'
    'Save' = 'Guardar'
    '5 · REQUIREMENTS' = '5 · REQUISITOS'
    'Analyze' = 'Analizar'
    'Install requirements' = 'Instalar requisitos'
    '6 · COMPONENTS' = '6 · COMPONENTES'
    'Startup' = 'Inicio'
    'Open the GUI when the node is running' = 'Abrir la GUI cuando el nodo está en marcha'
    'Open the voice on start' = 'Abrir la voz al iniciar'
    'The voice answers everything it hears (without saying "Archie")' = 'La voz responde a todo lo que oye (sin decir «Archie»)'
    'Desktop (shell)' = 'Escritorio (shell)'
    'Observatory' = 'Observatorio'
    'Draw with the CPU (if the GPU is busy or the GUI freezes)' = 'Dibujar con la CPU (si la GPU está ocupada o la GUI se congela)'
    'Seconds of warning before starting at Windows sign-in' = 'Segundos de aviso antes de iniciar al entrar a Windows'
    'Setup wizard (voice and key)' = 'Asistente de instalación (voz y clave)'
    'Open logs' = 'Abrir registros'
    'Uninstall the launcher' = 'Desinstalar el lanzador'
    'Target' = 'Destino'
    'WSL distribution' = 'Distribución de WSL'
    'VirtualBox machine' = 'Máquina de VirtualBox'
    'Log' = 'Registro'
    'Error: {0} (line {1})' = 'Error: {0} (línea {1})'
    'Core (archied)' = 'Core (archied)'
    'Running' = 'En marcha'
    'Stopped' = 'Detenido'
    'Partly running' = 'En marcha a medias'
    'No target' = 'Sin destino'
    'Starting…' = 'Iniciando…'
    'Restarting…' = 'Reiniciando…'
    'Stopping…' = 'Deteniendo…'
    'Switching model…' = 'Cambiando de modelo…'
    'Opening the GUI…' = 'Abriendo la GUI…'
    'Freeing space…' = 'Liberando espacio…'
    'VirtualBox · {0}' = 'VirtualBox · {0}'
    'WSL · {0}' = 'WSL · {0}'
    'Everything responds. Restart shuts it down and brings it up again.' = 'Todo responde. Reiniciar lo apaga y lo vuelve a levantar.'
    'Nothing is running yet. Start brings up the database, the local model, Core, Wanda and the view.' = 'Nada corre todavía. Iniciar levanta la base de datos, el modelo local, Core, Wanda y la vista.'
    'Some services do not respond (below). Restart brings them up again.' = 'Algunos servicios no responden (abajo). Reiniciar los vuelve a levantar.'
    'Cannot find the distribution or the virtual machine. Choose it in Advanced → Target.' = 'No encuentro la distribución o la máquina virtual. Elígela en Avanzado → Destino.'
    'Restart' = 'Reiniciar'
    'Local model · {0}' = 'Modelo local · {0}'
    'Settings: {0} · Start with Windows: {1}' = 'Ajustes: {0} · Inicio con Windows: {1}'
    'The WSL disk takes {0} on {1}: and Linux uses {2}. Freeing space returns about {3} to Windows (it stops the node for a few minutes).' = 'El disco de WSL ocupa {0} en {1}: y Linux usa {2}. Liberar espacio devuelve unos {3} a Windows (detiene el nodo unos minutos).'
    'Only {0} left on {1}:. With a full disk WSL freezes; free up space in Windows or compact the WSL disk.' = 'Quedan {0} libres en {1}:. Con el disco lleno WSL se congela; libera espacio en Windows o compacta el disco de WSL.'
    'VirtualBox: {0}' = 'VirtualBox: {0}'
    'WSL: {0}' = 'WSL: {0}'
    '{0} · {1:N1} of {2:N1} GB in use' = '{0} · {1:N1} de {2:N1} GB en uso'
    'WSL memory' = 'Memoria de WSL'
    'Models in' = 'Modelos en'
    '{0} · {1} truly free' = '{0} · {1} libres de verdad'
    'not configured' = 'no configurada'
    'Voices' = 'Voces'
    'Listening' = 'Escucha'
    'Spanish and English (automatic); it answers in the language it hears' = 'español e inglés (auto); responde en el idioma que oye'
    'AI pause' = 'Pausa de la IA'
    'paused' = 'en pausa'
    'no' = 'no'
    'no reading' = 'sin lectura'
    'On start' = 'Al iniciar'
    'opens the GUI ({0})' = 'abre la GUI ({0})'
    'does not open the GUI' = 'no abre la GUI'
    ' and the voice' = ' y la voz'
    'Start with Windows' = 'Inicio con Windows'
    'yes' = 'sí'
    'The rest appears when the node responds.' = 'El resto aparece cuando el nodo responde.'
    'This runtime does not support it yet' = 'Este runtime no lo soporta todavía'
    'Fits in the unified memory' = 'Cabe en la memoria unificada'
    'Does not fit on this computer' = 'No cabe en este equipo'
    'Fits on the GPU' = 'Cabe en la GPU'
    'GPU and CPU, slower' = 'GPU y CPU, más lento'
    'CPU only, slow' = 'Solo CPU, lento'
    'The list appears when the distribution responds (Start).' = 'La lista aparece cuando la distribución responde (Iniciar).'
    '{0:N0} GB of unified memory' = '{0:N0} GB de memoria unificada'
    '{0:N1} GB of GPU · {1:N1} GB of WSL memory' = '{0:N1} GB de GPU · {1:N1} GB de memoria en WSL'
    'answers on :{0}' = 'responde en :{0}'
    'on this computer' = 'en este equipo'
    'downloading · {0:P0}' = 'descargando · {0:P0}'
    'not downloaded' = 'no descargado'
    ' · license {0}' = ' · licencia {0}'
    ' · {0} parts' = ' · {0} partes'
    'In use' = 'En uso'
    'Use' = 'Usar'
    'Delete' = 'Borrar'
    'Downloading…' = 'Descargando…'
    'No space' = 'Sin espacio'
    'Resume download' = 'Continuar descarga'
    'Download' = 'Descargar'
    'One answers at a time. {0} · {1} · {2} free in the models folder.{3}' = 'Uno responde a la vez. {0} · {1} · {2} libres en la carpeta de modelos.{3}'
    ' {0} more do not fit on this computer.' = ' {0} más no caben en este equipo.'
    'Could not switch to {0}.\n\n{1}' = 'No se pudo cambiar a {0}.

{1}'
    'The download of {0} did not finish. It can continue where it left off.\n\n{1}' = 'La descarga de {0} no terminó. Se puede continuar donde quedó.

{1}'
    '{0} is downloaded. Use makes it answer.' = '{0} está descargado. Usar lo pone a responder.'
    'Delete {0} from this computer? It can be downloaded again.' = '¿Borrar {0} de este equipo? Se puede volver a descargar.'
    'The voice is already open.' = 'La voz ya está abierta.'
    'Local model on Windows: {0} on :{1}' = 'Modelo local en Windows: {0} en :{1}'
    'The local model did not start: {0}' = 'El modelo local no arrancó: {0}'
    'archie-node {0} in {1}' = 'archie-node {0} en {1}'
    'A service did not come up: see above.' = 'Algún servicio no levantó: ver arriba.'
    'Cannot find {0}.' = 'No encuentro {0}.'
    'Freeing space stops the node and WSL for a few minutes, compacts its disk and starts it again. Windows will ask for administrator permission.\n\nContinue?' = 'Liberar espacio detiene el nodo y WSL unos minutos, compacta su disco y lo vuelve a iniciar. Windows pedirá permiso de administrador.

¿Seguir?'
    'Freeing space: compacting the WSL disk' = 'Liberando espacio: compactando el disco de WSL'
    'Administrator permission was not given: nothing was compacted.' = 'No se dio el permiso de administrador: no se compactó nada.'
    'Compaction finished. {0}: has {1} free.' = 'Compactación terminada. {0}: tiene {1} libres.'
    'The folder is a Linux path, like /mnt/d/ArchieLocal/models.' = 'La carpeta es una ruta de Linux, como /mnt/d/ArchieLocal/models.'
    'Not installed' = 'Sin instalar'
    'Checking…' = 'Comprobando…'
    'Missing requirements' = 'Faltan requisitos'
    'Verifying…' = 'Verificando…'
    'Installing…' = 'Instalando…'
    'Restart Windows' = 'Reinicia Windows'
    'ArchieOS installed' = 'ArchieOS instalado'
    'Error' = 'Error'
    'Needs repair' = 'Necesita reparación'
    'The installer ended without a response (code {0}).' = 'El instalador terminó sin respuesta (código {0}).'
    'Choose a folder (a full path, like X:\ArchieOS).' = 'Elige una carpeta (una ruta completa, como X:\ArchieOS).'
    'On {0} {1} remain · ' = 'En {0} quedan {1} · '
    'Drive {0} is not available · ' = 'La unidad {0} no está disponible · '
    '{0}{1} needed (download {2} + distribution {3}, release estimate).\nIt will be: {4}\downloads\{5} · {4}\wsl\{6}\ext4.vhdx · {4}\state · {4}\logs' = '{0}hacen falta {1} (descarga {2} + distribución {3}, estimado del release).
Quedará: {4}\downloads\{5} · {4}\wsl\{6}\ext4.vhdx · {4}\state · {4}\logs'
    'That folder cannot be used: {0}' = 'No se puede usar esa carpeta: {0}'
    'Installer not available' = 'Instalador no disponible'
    'The files ArchieOS-Installer.ps1 and archieos-manifest.json are missing next to the launcher.' = 'Faltan ArchieOS-Installer.ps1 y archieos-manifest.json junto al lanzador.'
    'Installation interrupted' = 'Instalación interrumpida'
    'It was cut off halfway. Continue installation picks up where it left off (the download resumes).' = 'Se cortó a medias. Continuar instalación sigue donde quedó (la descarga se reanuda).'
    'Downloads ArchieOS from its release, checks it and installs it in WSL 2.' = 'Descarga ArchieOS desde su release, lo comprueba y lo instala en WSL 2.'
    'Advanced → Install shows what is missing and how to fix it.' = 'Avanzado → Instalar muestra qué falta y cómo resolverlo.'
    'WSL was installed: restart Windows and open the launcher again to continue.' = 'WSL se instaló: reinicia Windows y vuelve a abrir el lanzador para seguir.'
    'ArchieOS running' = 'ArchieOS en ejecución'
    'Continue installation' = 'Continuar instalación'
    'Windows remains the host; ArchieOS (Arch Linux) runs in WSL 2 with its own logic, permissions and models. ' = 'Windows sigue siendo el anfitrión; ArchieOS (Arch Linux) corre en WSL 2 con su propia lógica, permisos y modelos. '
    'The image is in a private repository on {0}: a token from an account with read access is needed. {1}' = 'La imagen está en un repositorio privado de {0}: hace falta un token de una cuenta con acceso de lectura. {1}'
    'A token is saved, encrypted for your Windows user (DPAPI).' = 'Hay un token guardado, cifrado para tu usuario de Windows (DPAPI).'
    'There is none saved.' = 'No hay ninguno guardado.'
    'The download is public: no token needed.' = 'La descarga es pública: no hace falta token.'
    'inside {0}; it is checked with archie-offline --check' = 'dentro de {0}; se comprueba con archie-offline --check'
    'downloaded and checked' = 'descargado y comprobado'
    'downloaded; it will be checked' = 'descargado; se comprobará'
    'half downloaded · {0:P0}' = 'descarga a medias · {0:P0}'
    'checked inside the installation' = 'comprobado dentro de la instalación'
    'arrives with the image' = 'llega con la imagen'
    'Analyze checks this computer: nothing is changed without asking.' = 'Analizar revisa este equipo: nada se cambia sin preguntar.'
    ' · {0} left' = ' · quedan {0}'
    'Downloading {0} · {1} of {2} · {3}/s{4}' = 'Descargando {0} · {1} de {2} · {3}/s{4}'
    'Verifying the image format' = 'Verificando el formato de la imagen'
    'Importing into WSL and checking each model (a few minutes)' = 'Importando en WSL y comprobando cada modelo (unos minutos)'
    'Install: analyzing this computer' = 'Instalar: analizando este equipo'
    'Install: {0}' = 'Instalar: {0}'
    '{0} · build {1} · {2} · {3} · {4}' = '{0} · compilación {1} · {2} · {3} · {4}'
    'no WSL' = 'sin WSL'
    'as administrator' = 'como administrador'
    'without administrator rights (requested only to install requirements)' = 'sin permisos de administrador (se piden solo para instalar requisitos)'
    'Install: {0} requirements are missing' = 'Instalar: faltan {0} requisitos'
    'Install: this computer meets the requirements' = 'Instalar: el equipo cumple los requisitos'
    'This will run, with administrator permission (Windows will ask):\n\n{0}\n\n{1}Continue?' = 'Se va a ejecutar, con permiso de administrador (Windows lo pedirá):

{0}

{1}¿Seguir?'
    'After that Windows has to restart; the launcher continues where it left off.\n\n' = 'Después hay que reiniciar Windows; el lanzador sigue donde quedó.

'
    'WSL is installed. Restart Windows and open the launcher again to continue the installation.' = 'WSL está instalado. Reinicia Windows y vuelve a abrir el lanzador para seguir con la instalación.'
    'Choose a folder with its full path, like X:\ArchieOS.' = 'Elige una carpeta con su ruta completa, como X:\ArchieOS.'
    'The distribution name has only letters, numbers, dot, hyphen and underscore.' = 'El nombre de la distribución lleva solo letras, números, punto, guion y guion bajo.'
    'The image is in a private repository on {0}. First save a token in Advanced → Install → Download access.' = 'La imagen está en un repositorio privado de {0}. Guarda primero un token en Avanzado → Instalar → Acceso a la descarga.'
    'Install ArchieOS {0}:\n\n• Download {1} ({2}) from {3}, with its SHA-256 checked, to {4}\downloads\n• Import it into WSL as {5}, with its disk in {4}\wsl\{5}\n• Prepare its keys and check each model (IRIS included)\n\nAbout {6} free are needed. No other distribution is touched. Continue?' = 'Instalar ArchieOS {0}:

• Descargar {1} ({2}) de {3}, con su SHA-256 comprobado, a {4}\downloads
• Importarla en WSL como {5}, con su disco en {4}\wsl\{5}
• Preparar sus claves y comprobar cada modelo (IRIS incluido)

Hacen falta unos {6} libres. No se toca ninguna otra distribución. ¿Seguir?'
    'Install: {0} ({1})' = 'Instalar: {0} ({1})'
    'ArchieOS is installed. Start ArchieOS brings it up.' = 'ArchieOS está instalado. Iniciar ArchieOS lo pone en marcha.'
    'ArchieOS installed.\n\nLocation: {0}\nVersion: {1}\nDistribution: {2}' = 'ArchieOS instalado.

Ubicación: {0}
Versión: {1}
Distribución: {2}'
    'The installation did not finish.\n\n{0}\n\nLog: {1}' = 'La instalación no terminó.

{0}

Registro: {1}'
    '{0}\n\nReuse that distribution? It is first checked with its own archie-offline --check; nothing is imported or deleted.\n\n(No: change the distribution name in Advanced → Install to install separately.)' = '{0}

¿Reutilizar esa distribución? Antes se comprueba con su propio archie-offline --check; no se importa ni se borra nada.

(No: cambia el nombre de la distribución en Avanzado → Instalar para instalar aparte.)'
    'It cannot be reused: {0}' = 'No se puede reutilizar: {0}'
    '{0} is registered in WSL, but its disk no longer exists:\n{1}\n\nRepair removes that registration (wsl --unregister {0}; nothing is left to lose) and imports the downloaded image again. Continue?' = '{0} está registrada en WSL, pero su disco ya no existe:
{1}

Reparar quita ese registro (wsl --unregister {0}; no queda nada que perder) y vuelve a importar la imagen descargada. ¿Seguir?'
    'Repair prepares the distribution again (keys and state folder) and checks each model; if it is missing, it re-imports it from the downloaded image. It deletes no data. Continue?' = 'Reparar vuelve a preparar la distribución (claves y carpeta de estado) y comprueba cada modelo; si falta, la reimporta desde la imagen descargada. No borra datos. ¿Seguir?'
    'Repaired: the installation passes its check.' = 'Reparado: la instalación pasa su comprobación.'
    'It could not be repaired.\n\n{0}' = 'No se pudo reparar.

{0}'
    'Install: cancelling (what was downloaded is kept)' = 'Instalar: cancelando (lo descargado se conserva)'
    'Folder for ArchieOS: the download, the WSL disk, its state and its logs' = 'Carpeta para ArchieOS: la descarga, el disco de WSL, su estado y sus registros'
    'Install: {0} token saved (encrypted with DPAPI)' = 'Instalar: token de {0} guardado (cifrado con DPAPI)'
    'Install: token forgotten' = 'Instalar: token olvidado'
    'ArchieOS was already running.' = 'ArchieOS ya estaba en ejecución.'
    'ArchieOS running: {0}' = 'ArchieOS en ejecución: {0}'
    'ArchieOS did not start.\n\n{0}' = 'ArchieOS no arrancó.

{0}'
    'Verify checks the SHA-256 of each model inside the installation.\n\nAlso load IRIS on the CPU to check that its weights can be read? (up to 10 minutes)' = 'Verificar comprueba el SHA-256 de cada modelo dentro de la instalación.

¿Cargar también IRIS en la CPU para comprobar que sus pesos se leen? (hasta 10 minutos)'
    'Installation correct: {0} models checked (source {1}).{2}' = 'Instalación correcta: {0} modelos comprobados (código {1}).{2}'
    'The verification failed.\n\n{0}\n\nRepair?' = 'La verificación falló.

{0}

¿Reparar?'
    'VirtualBox is not installed.' = 'VirtualBox no está instalado.'
    'VirtualBox is installed but has no machines. The ArchieOS OVA image is not published yet; when it exists, you will choose it here and turn it on, off or restart it.' = 'VirtualBox está instalado pero no tiene máquinas. La imagen OVA de ArchieOS aún no se publica; cuando exista, aquí se elige y se enciende, apaga o reinicia.'
    'In VirtualBox the launcher turns the machine on, off and restarts it; Open GUI shows its screen. The image itself brings up the services when it boots.' = 'En VirtualBox el lanzador enciende, apaga y reinicia la máquina; Abrir GUI muestra su pantalla. Los servicios los levanta la propia imagen al arrancar.'
    'ArchieOS: brings the node up when you sign in to Windows' = 'ArchieOS: levanta el nodo al entrar a Windows'
    'Uninstall the launcher? Its shortcuts, start with Windows and settings are removed. The WSL distribution, the node and the models stay.' = '¿Desinstalar el lanzador? Se quitan sus accesos directos, el inicio con Windows y sus ajustes. La distribución de WSL, el nodo y los modelos se quedan.'
    'Open ArchieOS' = 'Abrir ArchieOS'
    'Restart the node' = 'Reiniciar el nodo'
    'Stop the node' = 'Detener el nodo'
    'Quit the launcher' = 'Salir del lanzador'
    'It keeps running. The tray icon opens it again.' = 'Sigue en marcha. El icono de la bandeja lo vuelve a abrir.'
    'Start with Windows: bringing the node up' = 'Inicio con Windows: levantando el nodo'
    'ArchieOS starts in {0} s' = 'ArchieOS se inicia en {0} s'
    'Start with Windows: it did not start this time' = 'Inicio con Windows: no se inició esta vez'
    'Start with Windows turned off. It is turned on again in the panel or in Advanced → Startup.' = 'Inicio con Windows desactivado. Se vuelve a activar en el panel o en Avanzado → Inicio.'
    '{0} shares {1:N0} GB of memory with the CPU' = '{0} comparte {1:N0} GB de memoria con la CPU'
    '{0} with {1:N0} GB' = '{0} con {1:N0} GB'
    'no GPU the runtime can use: a small model on the CPU' = 'sin GPU que el runtime use: un modelo pequeño en la CPU'
    'ArchieOS has no ARM image yet: this computer cannot install it yet' = 'ArchieOS aún no tiene imagen para ARM: este equipo no puede instalarlo todavía'
    'Downloading {0}' = 'Descargando {0}'
    'Could not download {0}' = 'No se pudo descargar {0}'
    '{0} did not arrive complete; try again' = '{0} no llegó completo; vuelve a intentarlo'
    'The {0} runtime has no llama-server.exe' = 'El runtime {0} no trae llama-server.exe'
    'Importing {0} into VirtualBox' = 'Importando {0} en VirtualBox'
    'VirtualBox could not import the machine' = 'VirtualBox no pudo importar la máquina'
}

# The language: the one asked for, else ARCHIE_LANG, else the launcher's setting, else English.
function Initialize-Language([string]$requested = '') {
    $code = $requested
    if (-not $code) { $code = $env:ARCHIE_LANG }
    if (-not $code) {
        try {
            $file = Join-Path $env:APPDATA 'ArchieOS\launcher.json'
            if (Test-Path -LiteralPath $file) { $code = [string](Get-Content -LiteralPath $file -Raw -Encoding UTF8 | ConvertFrom-Json).lang }
        } catch { }
    }
    $code = ([string]$code).ToLowerInvariant()
    if ($code.Length -gt 2) { $code = $code.Substring(0, 2) }
    $script:UiLang = if ($code -and $script:Languages.Contains($code)) { $code } else { 'en' }
}
function T([string]$text) {
    $t = $text
    if ($script:UiLang -ne 'en') {
        $table = $script:I18n[$script:UiLang]
        if ($table -and $table.ContainsKey($text)) { $t = $table[$text] }
    }
    $t = $t.Replace('\n', "`n")
    if ($args.Count) { return ($t -f $args) }
    return $t
}
# The window's own texts are written in English in the XAML; once it is built they are looked up
# here (a text, a button, a tab header or a tooltip), whatever the language.
function Update-UiLanguage($root) {
    if ($script:UiLang -eq 'en') { return }
    $table = $script:I18n[$script:UiLang]
    if (-not $table) { return }
    $stack = New-Object System.Collections.Stack
    $stack.Push($root)
    while ($stack.Count) {
        $o = $stack.Pop()
        if ($o -is [System.Windows.Controls.TextBlock]) {
            if ($o.Text -and $table.ContainsKey($o.Text)) { $o.Text = $table[$o.Text] }
        } elseif ($o -is [System.Windows.Controls.HeaderedContentControl]) {
            if ($o.Header -is [string] -and $table.ContainsKey($o.Header)) { $o.Header = $table[$o.Header] }
        } elseif ($o -is [System.Windows.Controls.ContentControl]) {
            if ($o.Content -is [string] -and $table.ContainsKey($o.Content)) { $o.Content = $table[$o.Content] }
        }
        if ($o -is [System.Windows.FrameworkElement] -and $o.ToolTip -is [string] -and $table.ContainsKey($o.ToolTip)) { $o.ToolTip = $table[$o.ToolTip] }
        if ($o -is [System.Windows.DependencyObject]) {
            foreach ($child in [System.Windows.LogicalTreeHelper]::GetChildren($o)) { if ($child -is [System.Windows.DependencyObject]) { $stack.Push($child) } }
        }
    }
}
}
