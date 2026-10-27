# ArchieOS WSL con Bonsai y diseño Xavi

Esta variante se distribuye como **un único archivo `.wsl`**, con Bonsai2 9B
PTQ1_0, reconocimiento de voz, voces Piper, pesos de IRIS, caché de sus encoders
y runtimes incluidos. No requiere descargar pesos durante la instalación.

Requiere Windows con WSLg y **WSL2.4.4 o posterior**, que proporciona el instalador
de archivos `.wsl` por doble clic. WSL es un componente de Windows y debe estar
instalado previamente. Reserva al menos40GB libres para la distribución y8GB de
RAM para WSL. El tamaño final de descarga figura en el release.

1. Descarga el `.wsl` completo desde el enlace externo del release.
2. Comprueba su SHA-256 con el indicado allí y haz doble clic.
3. WSL registra `ArchieOS-Bonsai-Xavi`; el primer inicio prepara claves y una base
   de datos nuevas, comprueba los pesos y abre el escritorio Polar Prisma.
4. Di «Archie» para hablar. También puedes abrir **ArchieOS** desde las aplicaciones
   de Windows integradas por WSLg, o ejecutar `archie-offline` dentro de la distribución.

El arranque predeterminado usa CPU y no inicia IRIS/Stage ni trabajos CUDA. Evita
competir con un entrenamiento existente. La voz local puede tardar varios minutos:
una prueba del prompt completo tardó258s en el equipo de construcción. El límite
configurado es600s; no es una promesa de latencia interactiva ni de precisión.

IRIS, Stage y sus comandos conservan sus políticas y permisos existentes. Incluir
sus pesos no certifica sus acciones ni elimina las configuraciones que requieran
servicios opcionales. El escritorio no otorga autoridad de administración nueva.

Cada instalación genera su identidad y su base de datos. La imagen no contiene la
base de datos del desarrollador, claves de cuenta, conversaciones, logs, checkpoints
de entrenamiento ni adaptadores Wanda. Los puertos ocupados por otros servicios
se respetan: el arranque falla con un diagnóstico en vez de reemplazarlos.

El keyring local de `pacman` también se genera en el primer inicio, desde las
claves públicas de Arch incluidas. El instalador por importación ejecuta la misma
inicialización como root antes de la configuración de usuario. Volver a ejecutar
ese paso conserva la identidad local. Los empaquetadores rechazan un rootfs que
todavía contenga el keyring del constructor, incluida su base de confianza.

Para comprobar la integridad sin iniciar servicios: `archie-offline --check`.
Los logs de tu instalación se guardan en `~/.local/state/archie-offline/`.

La compilación se mantiene como borrador hasta verificar el artefacto final. La
instalación moderna por doble clic no se ha comprobado en el host constructor,
que utiliza WSL2.2.4 y no se reinició para preservar el entrenamiento activo.
Las diferencias de fidelidad visual y pruebas de GPU/ISO/VBox pendientes del PR41
siguen documentadas en `_dev/verification/xavi/source-port.md`.

Formato de instalación: [documentación oficial de WSL](https://learn.microsoft.com/en-us/windows/wsl/build-custom-distro).
