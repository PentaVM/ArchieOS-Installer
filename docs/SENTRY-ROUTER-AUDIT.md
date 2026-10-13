# Sentry: router, captura y pruebas de seguridad autorizadas

Auditoría del 2026-10-13, rama `codex/sentry-discovery`, Draft PR39. La selección
del router procede del inventario de Sentry, no del inventario de dispositivos de
Windows ni de una dirección/modelo configurados para esta casa. Identificadores,
salidas completas y capturas quedan en almacenamiento privado, fuera del repositorio.

## Lo observado

- Un anuncio SSDP identifica un MitraStar GPT-2741GNAC. Es una declaración del
  dispositivo; no proporciona credenciales ni la versión exacta del firmware.
- Diagnóstico limitado a ese router y a TCP 22/80/443: los tres están abiertos.
  SSH responde como Dropbear 2019.78. Nmap llama `rtsp` al servicio de 80 y
  `ssl/https?` al de 443: son hipótesis del sondeo, no protocolos confirmados.
  El banner SSH tampoco establece qué firmware completo tiene el router.
- Nmap terminó con código 0 en 30,36 s. No se intentó autenticación ni se alteró
  la configuración del router.
- `archie-netwatch live` con TShark real sobre `eth0` de ArchieOS-Builder procesó
  208 tramas y salió con código 0 al detenerlo de forma controlada. Persistió dos
  registros MAC con 206 tramas asignadas y dos avisos `new-device`. Son extremos
  del enlace virtual de WSL; no dos dispositivos IoT identificados. La dirección
  del router aparece en tráfico que atraviesa ese enlace, no con su MAC física.
- El dueño confirmó que no hay credencial/clave SSH guardada en Vault. No hay
  canal administrativo autenticado ni evidencia de control del router.

## Implementaciones que ya existen

`core/crates/archie-sentry/src/adapters/router/` contiene `RouterAdapter`,
`probe_router`, clientes DHCP/vecinos, SSH e IGD. No hace falta otro controlador
paralelo ni un driver propio del modelo para hablar con esos protocolos.

- IGD utiliza SOAP `WANIPConnection:1` y permite altas/bajas de port mappings con
  lectura posterior de la entrada exacta. Es una capacidad limitada, no acceso root.
  Necesita una `control_url` configurada y validada; un anuncio SSDP no la habilita.
- SSH utiliza el binario del sistema, autenticación por clave, `BatchMode`,
  comprobación de host key y una lista fija de comandos con límites temporales.
  No admite un intérprete remoto arbitrario ni una contraseña pegada en argumentos.
- La sonda declara solamente canales/herramientas que contestan. Las actuaciones
  mantienen emparejamiento, Policy, leases, recursos y comprobaciones existentes.
- El controlador Core ya tiene cableado para `[sentry.router]`. Sus pruebas de
  protocolo y actuación no sustituyen una prueba autenticada en este router real.
  **Matiz de integración:** `RouterBinding::to_router_config` marca IGD/SSH según
  la configuración, y `operator_session` entrega esas capacidades al adaptador;
  no ejecuta `probe_router` en ese recorrido. `radio_tools` queda vacío, por lo que
  Wi-Fi sigue `Unsupported`. No presentar un canal configurado como canal sondeado.
  Falta conectar/verificar esa sonda en el ciclo del controlador.
- `archie-sentry-probe` expone únicamente la sonda IPP de impresoras. La sonda de
  router existe como biblioteca, pero todavía no como subcomando de esa herramienta.
- El descubrimiento LAN conserva metadatos de la descripción UPnP, pero no conecta
  automáticamente sus servicios/control URLs al adaptador IGD. Es un pendiente real.

## Opciones y decisión

| Opción | Qué aporta | Requisito y estado real |
| --- | --- | --- |
| Panel/SSH administrativo del router | Clientes, leases, vecinos y capacidades del firmware; control con recibos | Obtener acceso legítimo y configurar la referencia de clave; SSH abierto confirmado, login pendiente. Reutilizar el adaptador existente. |
| UPnP IGD | Estado WAN y operaciones de mapeo admitidas | Descubrir/validar el servicio y su control URL; verificar la respuesta real. No concede administración general. No se probó escritura. |
| TShark + Npcap en Windows | Observación desde una interfaz LAN del host | Driver instalado y captura del host verificada. En una LAN conmutada no garantiza tráfico entre otros equipos ni dispositivos aislados/dormidos. |
| `sshdump` + tcpdump en router/gateway | Captura en el punto donde pasan los paquetes | SSH autenticado, herramienta remota y privilegios de captura; no verificados en el router. Captura no implica contenido HTTPS/WhatsApp descifrado. |
| Gateway propio o sensor en puerto espejo/TAP | Cobertura del tráfico que atraviesa ese punto y observación persistente | Ubicación física y configuración de red. OPNsense dispone de API; soporte OpenWrt del modelo exacto no verificado. No recomendar flashear esta ONT basándose en su nombre. |
| Pruebas red team del router propio | Validación de superficie, segmentación, autenticación y vulnerabilidades | Alcance exacto y versión/configuración comprobadas. El modelo o un banner por sí solos no prueban que un exploit aplique. |

Recomendación: Sentry conserva un solo plano de autoridad en Core. Los colectores
locales/remotos producen observaciones acotadas con procedencia, alcance y antigüedad;
el feeder no transforma esos anuncios en autoridad ni revive datos de caché. Primero
activar el canal administrativo existente y confirmar clientes; para cobertura de
otros nodos, añadir captura desde un gateway o espejo. Npcap es el driver estándar
necesario para captura física en Windows; no un driver que concede control del router.

La [CVE-2023-33381 del registro oficial](https://github.com/CVEProject/cvelistV5/blob/main/cves/2023/33xxx/CVE-2023-33381.json)
describe inyección de comandos en la función ping **después de autenticarse**, para
firmware **AR_g5.8_110WVN0b7_2**. El firmware de esta casa es desconocido; no se ejecutó
el exploit. No hay evidencia de un takeover sin credenciales para este equipo.

## Herramientas verificadas

- **ArchieOS-Builder (Arch Linux):** TShark 4.7.4 ya estaba instalado. Se instalaron
  Nmap 7.991-1 y tcpdump 4.99.7-1 mediante pacman, con verificación de paquetes.
  La ISO ya declara `wireshark-cli`; esto no reconstruye ni certifica el release WSL.
- **Windows:** TShark 4.6.9 extraído del MSI oficial a una carpeta de usuario,
  añadido al PATH del usuario. Se verificaron SHA-256 y Authenticode antes de extraer.
  `tshark --version`, interfaces extcap y `sshdump --extcap-interfaces` funcionan.
  Tras instalar Npcap, Sentry enumera interfaces NPF del host y captura en la
  interfaz LAN correspondiente a su propio alcance de descubrimiento.
- **Npcap 1.89:** instalador oficial descargado, firma válida de Nmap Software LLC.
  Los primeros lanzamientos directos fallaron. **Instalación completada** mediante
  un lanzador Windows PowerShell elevado por `runas`: instalador salida 0,
  servicio `npcap` corriendo, TShark declara Npcap 1.89/libpcap 1.10.7.
  La captura posterior funciona desde la sesión normal del host. No se deshabilitó
  UAC ni se usaron switches silenciosos OEM de la edición gratuita.
- **Prueba local actual:**

  ```powershell
  $env:PATH="$env:LOCALAPPDATA\Archie\tools\wireshark-4.6.9;"+$env:PATH
  # Desde core/
  cargo test -p archie-lan --test tshark_e2e --locked -- --ignored --nocapture
  ```

  Resultado: 1 pasada, 0 fallos, 0 ignoradas. Usa un PCAP construido por el test;
  comprueba campos reales de TShark y alertas de suplantación ARP/IPv6, sospecha de
  túnel DNS y barrido. No demuestra un ataque real ni detección universal.

## Reproducción de la prueba de red

1. Ejecutar el colector LAN de Sentry y guardar inventario/diagnósticos en una
   carpeta privada. Seleccionar el router anunciado por SSDP en ese inventario;
   detenerse si es ambiguo o está fuera de la red privada autorizada.
2. Sobre la interfaz cuyo alcance se quiere verificar:

   ```sh
   archie-netwatch live --interface eth0 --state-dir PRIVATE_DIR/netwatch --learn 0 --seconds 60
   nmap -sT -sV --version-light -Pn -n -p 22,80,443 --max-rate 5 \
     --max-retries 1 --host-timeout 45s -oX PRIVATE_DIR/router-services.xml "$SENTRY_ROUTER"
   ```

   `SENTRY_ROUTER` se obtiene del inventario, no de una constante. La primera sesión
   usó timeout externo de 60 s para Nmap, esperó 10 s adicionales y detuvo el hijo
   TShark con SIGINT. El seguimiento añade `--seconds 1..86400`, que utiliza el
   autostop de TShark desde el inicio de captura y guarda el estado al terminar,
   incluso sin paquetes. No sustituye el timeout de inicio/proceso de Core ni
   instala un servicio. Sin ese flag conserva el modo continuo. Los límites
   inválidos/incompletos y opciones live desconocidas se rechazan antes de iniciar.
3. Ejecutar `archie-netwatch report --state-dir PRIVATE_DIR/netwatch`. Registrar
   interfaces, códigos de salida, tramas procesadas/asignadas, avisos y herramientas.
   Distinguir captura virtual, física y del gateway. No publicar direcciones/MACs.
4. La repetición con Npcap en el host está verificada abajo; repetir separadamente
   tras tener acceso al gateway.
   Medir cobertura contra dispositivos despiertos/dormidos conocidos por el dueño,
   CPU/RAM, pérdida de paquetes y propagación al feeder bajo Core. Evaluar segmentación
   con nodos controlados y ejercicios PCAP reproducibles antes de ensayos en vivo.

## Comparación de motores Windows — seguimiento

| Proyecto | Uso adecuado | Decisión para ArchieOS |
| --- | --- | --- |
| [Npcap](https://github.com/nmap/npcap) | Captura libpcap usada por TShark; interfaces locales Ethernet/Wi-Fi según hardware | Mantener para el backend existente. Versión 1.89 instalada y captura LAN del host verificada. |
| [WinDivert](https://github.com/basil00/WinDivert/wiki/WinDivert-Documentation) | Interceptar tráfico IP/flows/sockets del host o en tránsito; filtrar/modificar/reinyectar | Candidato distinto para control del tráfico del host. También exige administrador; su API de capa IP no sustituye la evidencia Ethernet/ARP del monitor actual. No instalado ni integrado. |
| [PcapPlusPlus](https://github.com/seladb/PcapPlusPlus) | API C++ multiplataforma de captura/parseo sobre varios motores | Útil si se mide la necesidad de un colector nativo C++. Envuelve Npcap/libpcap/WinDivert y otros; no elimina los requisitos del motor ni justifica sustituir TShark sin benchmark. |
| [Windows Packet Filter / NDISAPI](https://github.com/wiresock/ndisapi) | Captura/filtrado a nivel Ethernet con otro driver y API propia | Requiere otro backend y despliegue de driver. No hay benchmark ni integración que pruebe una ventaja para Sentry aquí. |
| [PktMon](https://learn.microsoft.com/en-us/windows-server/networking/technologies/pktmon/pktmon) + [etl2pcapng](https://github.com/microsoft/etl2pcapng) | Diagnóstico de red del host, trazas ETL y conversión a PCAPNG | Alternativa de diagnóstico ya incluida en Windows, sin instalar Npcap. En esta sesión `pktmon status` da acceso denegado; no resuelve la falta de elevación. No integrado como productor continuo. |

Verificación adicional: el lanzamiento directo del instalador Npcap devuelve
Win32 **740 (requiere elevación)** y Appinfo está activo. Se envió la solicitud
normal `runas` mediante `Shell.Application`, sin instalación observada en ese intento.
El lanzamiento posterior de Windows PowerShell con `runas` sí inició el instalador
y permitió verificar su éxito y el driver.
No se deshabilitó UAC ni se alteraron controles de Windows. Prueba del nuevo límite
con interfaz extcap `udpdump` y TShark real: duración de captura 2 s, proceso 4,65 s
incluyendo arranque/cierre, cero paquetes, salida 0, `state.json` y `told.json`
persistidos. Es prueba de terminación en inactividad, no captura de la LAN física.

**Prueba LAN posterior a la instalación:** la primera selección por nombre Wi-Fi
no produjo tramas. Sentry enumeró sus bindings IPv4 y su colector informó el alcance
actual; ese alcance correspondía a `Ethernet 6`, no Wi-Fi. Se cruzó el nombre con las
interfaces devueltas por `archie-netwatch interfaces`, sin constantes IP/MAC ni
inventario de dispositivos de Windows. Captura acotada de 20 s, mientras el propio
`archie-lan` ejecutaba descubrimiento de 6 s: **565 tramas procesadas**, **376 tramas
atribuidas** (89 480 bytes) a **4 registros MAC**, **4 avisos `new-device`/notice**,
ningún hostname confirmado, salida 0. No son cuatro IoT identificados ni cuatro
incidentes. El resto del tráfico no se convierte automáticamente en dispositivos
LAN. Los procesos de captura terminaron y sus datos permanecen privados.

Un lanzador temporal reportó erróneamente fallo al tratar el resumen normal de
stderr como error de Windows PowerShell 5; se corrigió el manejo del exit code.
Ese incidente no modifica el binario de Sentry ni se presenta como fallo del driver.
El runtime de Computer Use no pudo inicializarse (error de ruta 3); no se automatizó
el diálogo de elevación ni las pantallas del instalador mediante ese runtime.

Decisión: Npcap + TShark para la integración presente; evaluar PktMon como fallback
de diagnóstico y PcapPlusPlus para un colector nativo después de medir CPU, memoria,
latencia, pérdida y cobertura de campos. WinDivert sería otra capacidad de actuación,
bajo las autoridades existentes, no un reemplazo automático del observador.

## Fuentes primarias y siguientes integraciones

- [Wireshark: instalación Windows y componentes](https://www.wireshark.org/docs/wsug_html_chunked/ChBuildInstallWinInstall.html): sin Npcap permite leer archivos, no captura local; el instalador silencioso no instala Npcap.
- [Npcap: guía](https://npcap.com/guide/npcap-users-guide.html) y [descargas/licencia](https://npcap.com/): driver firmado, monitor mode dependiente del hardware, instalación silenciosa OEM. No redistribuir el instalador gratuito en el release de ArchieOS.
- [sshdump](https://www.wireshark.org/docs/man-pages/sshdump.html): captura remota autenticada con herramienta remota; ya disponible, falta validar endpoint/privilegios.
- [Nmap](https://github.com/nmap/nmap) y [referencia de opciones](https://nmap.org/book/man-briefoptions.html): diagnóstico externo acotado; no añadido como nuevo actuador de Core en esta sesión.
- [OPNsense API](https://docs.opnsense.org/development/api.html): candidato para gateway administrable con credenciales explícitas.
- [OpenWrt](https://github.com/openwrt/openwrt): candidato en hardware compatible; esta auditoría no confirmó soporte del MitraStar concreto.
- [Zeek](https://github.com/zeek/zeek) y [Suricata](https://github.com/OISF/suricata): futuros productores de observaciones IDS acotadas, no instalados/integrados aquí.

Pendientes concretos: credencial/clave administrativa; host key verificada;
sonda CLI y sonda real en el controlador; descubrimiento/validación de control URL IGD;
captura desde gateway; límites de duración/recurso; despliegue y prueba del feeder
bajo Core. Ninguno requiere duplicar Cortex ni sustituir las autoridades existentes.
