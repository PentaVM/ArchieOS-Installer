# Sentry: relevo para auditoría — 2026-10-11

De Claude para Codex. El dueño pidió que revises, audites y mejores esto con criterio
profesional. Está escrito para que puedas atacarlo: dice qué se hizo, qué se probó de
verdad, qué no, y dónde creo que están los defectos.

Repositorio único: `PentaVM/ArchieOS-Nvidia`. Rama `feat/sentry-complete`, sobre
`feat/sentry-port` (tu Sentry portado) y `fix/lan-limits`.

## Qué hay nuevo

| Pieza | Dónde | Qué hace |
|---|---|---|
| Captura | `core/crates/archie-lan/src/capture.rs` | Los 20 campos que se piden a TShark y el parser de cada línea |
| Auditoría | `core/crates/archie-lan/src/audit.rs` | Registro por dispositivo y seis reglas de alerta |
| Binario | `core/crates/archie-lan/src/bin/archie-netwatch.rs` | `interfaces`, `live`, `read`, `report` |
| Servicio | `distro/sentry/archie-netwatch.service`, `modules/sentry/netwatch.toml` | Unidad con `CAP_NET_RAW`/`CAP_NET_ADMIN`, no habilitada |
| Impresión | `core/crates/archie-sentry/src/adapters/ipp/print.rs` | `Print-Job`, `Get-Job-Attributes` |
| Páginas | `core/crates/archie-sentry/src/page.rs` | Texto a páginas A4 JPEG |
| Binario | `core/crates/archie-sentry/src/bin/archie-print.rs` | Imprime tras confirmación en el terminal |
| ISO | `distro/archiso/build.sh`, `packages.x86_64` | Instala los tres binarios y `wireshark-cli` |

Documentos: `docs/SENTRY-NETWATCH.md`, `core/crates/archie-sentry/README.md` (sección Imprimir).

## Qué se probó y cómo

- **Unitarias**: `archie-lan` 30, `archie-sentry` 40 (8 omitidas piden PostgreSQL). Windows y Linux.
- **Captura en vivo real** (ArchieOS-Builder, WSL2, TShark 4.7.4, root, `eth0`,
  `--learn 0`): 165 tramas. Con tráfico generado a propósito salieron `new-device` ×2,
  `port-scan` (131 puertos contra la puerta de enlace) y `cleartext-login` (Telnet).
  `state.json` y `alerts.jsonl` quedaron en 0600, y el tráfico HTTPS a `example.com` se
  contó bajo su nombre de servidor.
- **Impresión contra impresora simulada** (el servidor TCP de `tests/adapters.rs`):
  codec, un solo envío del documento, seguimiento hasta `completed`, rechazo de
  respuestas truncadas, de otro `request-id` y de estados inexistentes.
- **Página**: una página de prueba se generó y se miró; el español se lee.

## Qué NO se probó

1. **Ninguna impresora real.** La Brother DCP-T720DW no respondía en la red (ni 631, ni
   80, ni mDNS). No sé si acepta `image/jpeg`, ni si escala la página a A4, ni si
   conserva el trabajo el tiempo suficiente para leer `completed`.
2. **`dns-tunnel-suspect` en vivo.** En WSL el DNS sale por loopback, no por `eth0`; la
   regla sólo tiene prueba unitaria.
3. **`address-moved` y `host-sweep` en vivo.** Sólo unitarias.
4. **La unidad systemd y la ISO.** `build.sh` pasó `bash -n` y nada más. No sé si
   `DynamicUser` con capacidades ambientales alcanza para que `dumpcap` abra la interfaz
   en Arch, donde `dumpcap` se instala con grupo `wireshark` y file capabilities.
5. **`read` con un pcap real.** El camino es el mismo que `live` salvo `-r`; no se corrió.
6. **Carga.** Nada medido: tramas por segundo, memoria con miles de dispositivos, coste
   de un proceso TShark permanente.

## Dónde creo que hay defectos

Ordenados por lo que más me preocupa.

1. **Imprimir no pasa por el motor de Sentry.** `archie-print` es una herramienta de
   operador que habla IPP directamente: no hay `Intent`, ni policy, ni journal, ni
   aprobación por digest, ni recurso emparejado. Lo hice así porque el motor exige
   `deadline` ≤ 30 s y una postcondición medida, y un trabajo de impresión tarda más y
   su único "sensor" es lo que la impresora declara. La integración correcta
   probablemente es `Command::Print { sha256, format, pages }` con postcondición «la
   impresora tiene el trabajo N con ese nombre», y `completed` como observación
   posterior fuera del deadline. Es tu motor: decide tú la forma.
2. **`archie-print` acepta la URL por argumento.** Core, en cambio, sólo usa bindings de
   `archie.toml`. La herramienta valida que sea IP privada y admite `--expect-uuid`,
   pero el UUID es autodeclarado. No es alcanzable desde un modelo, pero tampoco está
   atada a la configuración.
3. **La confirmación es escribir `print`.** No es tu confirmación por digest con UID,
   PID y terminal en primer plano. Cualquier proceso con stdin puede escribirla.
4. **El estado de la auditoría se confía al leerlo.** `state.json` se deserializa sin
   límites: un archivo manipulado con millones de entradas se carga entero. Los topes
   (`MAX_DEVICES`, `MAX_NAMES`, `MAX_PEERS`) sólo se aplican al insertar.
5. **Deduplicación de alertas en memoria.** `told` se vacía al llegar a 16 384 claves y
   se pierde al reiniciar: tras un reinicio se repiten alertas ya dadas. `address-moved`
   sólo avisa una vez por par dirección/dispositivo, así que un envenenamiento ARP que
   alterna entre dos MAC produce dos alertas y luego silencio.
6. **Umbrales sin calibrar.** 100 puertos, 50 hosts, 60 s, entropía 4,0, etiqueta ≥ 40:
   son criterio mío, no medidos contra tráfico real. Espera falsos positivos (un
   escáner de vulnerabilidades legítimo, clientes BitTorrent, CDNs con nombres largos)
   y falsos negativos (escaneo lento, túnel con etiquetas cortas).
7. **El primer paquete a un destino se cuenta bajo su IP.** El nombre del servidor se
   aprende en el ClientHello, que llega después del SYN: en `sent_to` aparecen ambos.
8. **mDNS cuenta como pregunta DNS.** `asked` mezcla nombres `.local` con dominios.
9. **TShark como dependencia en ejecución.** Si muere, `live` termina con error y
   systemd lo reinicia a los 30 s; lo capturado en ese hueco se pierde. Los nombres de
   campo cambian entre versiones de Wireshark (`dhcp.option.hostname` fue `bootp.…`), y
   los booleanos se imprimen `1` o `True` según versión; el parser acepta ambos, pero
   un campo renombrado deja de leerse en silencio.
10. **IPv6.** `address-moved` sólo mira ARP; no hay equivalente para Neighbor Discovery.
11. **`page.rs`.** Tipografía 8×8 ampliada: legible y fea. Sin negrita, sin tamaños, sin
    imágenes. Un carácter fuera de Latin-1 sale como `?`.
12. **Dos dependencias nuevas**: `font8x8` y `jpeg-encoder`. Sin revisar licencias ni
    MSRV contra la política del workspace.

## Lo que falta para llamar «terminado» a Sentry

No está terminado. Faltan, como mínimo:

- Unir `archie-lan` (inventario activo), `archie-netwatch` (registro pasivo) y tu
  `passive_network` en una sola vista de dispositivos, y llevarla al world graph. Hoy
  son tres fuentes con tres formatos.
- Producir `PillarContext` desde alertas reales y pasarlo por Cerebra.
- Respuesta: hoy todo es observación. No hay bloqueo, cuarentena ni aviso por voz.
- Adaptador del router (DHCP, vecinos, Wi-Fi) según su firmware real. Sin eso, o sin
  puerto espejo, un nodo conmutado sólo ve su propio tráfico.
- Baselines por dispositivo (volumen, destinos habituales) y alertas por desviación.
- Control de dispositivos más allá de Home Assistant: Roku ECP está leído, no actuado;
  nada de Matter, MQTT, ONVIF ni Chromecast.
- Impresión dentro del motor (punto 1) y verificada en una impresora real.

## Cómo reproducir

```bash
cd core
cargo test -p archie-lan -p archie-sentry
```

En Linux, con `wireshark-cli` instalado y como root:

```bash
archie-netwatch live --interface eth0 --state-dir /tmp/nw --learn 0
archie-netwatch report --state-dir /tmp/nw
```

Con la impresora encendida:

```bash
archie-sentry-probe ipp ipp://<IP>/ipp/print
archie-print --printer ipp://<IP>/ipp/print --text "Hola, soy Archie."
```

El primer comando dice qué formatos acepta; si `image/jpeg` no está, `archie-print` lo
dirá y no enviará nada.
