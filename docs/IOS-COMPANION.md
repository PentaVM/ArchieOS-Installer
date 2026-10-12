# iPhone: la app companion (modelo honesto)

Este documento explica qué puede y qué **no puede** hacer ArchieOS con un
iPhone, y por qué. La respuesta corta: iOS no permite control remoto
arbitrario del teléfono, y ArchieOS no finge lo contrario. Lo que sí existe
es una **app companion** instalada por el dueño, que recibe intenciones por
HTTPS local y ejecuta las superficies que Apple sí permite.

## Lo que iOS permite (y por tanto lo que el companion hace)

El sandbox de Apple limita a cualquier app — incluida la nuestra — a su
propio proceso. Dentro de ese límite, la app companion ofrece una superficie
real y útil:

1. **Abrir URLs y apps mediante URL schemes.** `music://`, `maps://`,
   `https://…`, o el scheme propio de cualquier app que lo declare. Desde
   ArchieOS esto es un `AppLaunch { app_id }` que se resuelve contra una
   tabla configurada por el dueño (`app_id` → URL); el comando nunca lleva
   una URL cruda, así que nada sin revisar llega a abrirse.

2. **Ejecutar Atajos (Shortcuts) mediante URLs `shortcuts://`.** Esta es la
   verdadera superficie de control en iOS. Un Atajo creado por el dueño
   puede, entre otras cosas:
   - activar/desactivar ajustes del dispositivo (modo avión, Wi-Fi, VPN,
     enfoque, brillo, bajo consumo…),
   - controlar accesorios **HomeKit** (luces, enchufes, termostato),
   - preparar y enviar mensajes **que el usuario confirma en pantalla**,
   - establecer el volumen (acción "Ajustar volumen" con un número como
     entrada),
   - obtener datos del dispositivo y devolverlos.

   El companion ejecuta el Atajo entregando la URL `shortcuts://run-shortcut`
   al sistema; lo que el Atajo hace después es entre el usuario e iOS.

3. **Reportar estado que iOS sí expone a una app:**
   - batería (`UIDevice`),
   - volumen de salida (`AVAudioSession.outputVolume`),
   - estado de enfoque (con el permiso explícito del usuario),
   - ubicación (solo con el permiso correspondiente concedido por el usuario),
   - el identificador de la última intención que el propio companion
     completó.

4. **Recibir intenciones disparadas por notificaciones push** cuando el
   companion no está en primer plano (flujo futuro; la entrega del push la
   hace Apple y el usuario puede exigir confirmación en el Atajo).

## Lo que iOS NO permite (y ArchieOS no hará jamás)

Dicho sin rodeos:

- **No se puede bloquear el teléfono remotamente.** No existe API pública
  para bloquear la pantalla. `SessionLock` en este adaptador devuelve
  siempre `Unsupported`. Cualquier producto que afirme bloquear un iPhone
  sin MDM está mintiendo, y con MDM la capacidad sigue siendo del perfil de
  administración, no de una app.
- **No se puede leer el contenido de otras apps** (mensajes, pantalla,
  portapapeles ajeno, historial). Cada app vive en su sandbox.
- **No se puede saber qué app está en primer plano.** Por eso el campo
  `foreground_app` del contrato se informa aquí como *la última acción que
  el companion ejecutó* — lo que la app hizo, no lo que el teléfono muestra.
  La documentación del adaptador y los recibos lo declaran así.
- **No se puede controlar el sistema arbitrariamente** (apagar, reiniciar,
  cambiar ajustes sin un Atajo, simular toques). Fuera de los Atajos que el
  dueño cree, no hay superficie.
- **Nada sale a la nube.** Las intenciones viajan por HTTPS local dentro de
  la LAN del dueño. Las notificaciones push (cuando se usen) solo llevan un
  identificador opaco; el contenido se consulta después por el canal local.

## Arquitectura

```
archie-core (Intent → policy → aprobación)
      │
      ▼  HTTPS local, certificado fijado + token
adapters::ios_companion  ──POST /v1/intent──▶  App companion (iPhone)
        │                  GET /v1/state  ◀──  (batería, volumen, enfoque,
        ▼                                       última acción)
   postcondición medida = ack de la app + reporte de estado
```

- **Canal:** HTTPS con el certificado autofirmado del companion **fijado en
  la configuración** (es la única raíz de confianza de esa conexión) más un
  token bearer. Sin certificado fijado no hay HTTPS fuera de loopback; HTTP
  solo se acepta en loopback (desarrollo/pruebas).
- **Intenciones:** `POST /v1/intent` con `{id, kind: open_url|shortcut,
  target, input?}`. El ack de la app es la postcondición de despacho.
- **Estado:** `GET /v1/state`. Los campos que iOS no expone simplemente
  faltan; el adaptador nunca inventa valores.
- **App inalcanzable** → error honesto (`Transport`/`Unavailable`), nunca un
  estado cacheado.

## Configuración (lado servidor)

```text
base:                  https://192.168.1.70:8443
token:                 <token generado por el companion>
pinned_certificate_pem: <PEM del certificado del companion>
devices:
  iphone:
    actions:
      abrir_mapas:  { kind: open_url, target: "maps://" }
      lampara:      { kind: shortcut,  target: "Alternar lámpara" }
    volume_shortcut: "Ajustar volumen Archie"
```

- `abrir_mapas` / `lampara` son los `app_id` que un `Intent` puede nombrar;
  la tabla es también la lista de permitidos.
- `volume_shortcut` nombra un Atajo creado por el dueño que recibe un número
  (porcentaje) y ejecuta "Ajustar volumen". Si no está configurado,
  `VolumeSet` es `Unsupported`.

## Matriz honesta de capacidades (iPhone)

| Capacidad                         | Estado     | Base real                                   |
|-----------------------------------|------------|---------------------------------------------|
| Abrir app/URL (`AppLaunch`)       | Soportado  | URL schemes, tabla configurada              |
| Ejecutar Atajo (vía `AppLaunch`)  | Soportado  | `shortcuts://run-shortcut`                  |
| Volumen (`VolumeSet`)             | Condicional| Solo con `volume_shortcut` + lectura AVAudioSession |
| Batería / enfoque / ubicación     | Reporte    | APIs públicas con permiso del usuario       |
| Bloquear el teléfono              | **No**     | iOS no lo expone; `Unsupported` siempre     |
| App en primer plano real          | **No**     | iOS no lo expone; se reporta la última acción de la app |
| Leer datos de otras apps          | **No**     | Sandbox de Apple                            |
| Apagar / reiniciar / simular toques | **No**   | No existe superficie pública                |

Para la matriz equivalente de PCs (Windows/Linux/macOS por SSH), véase la
documentación del módulo `adapters::host_ssh`.
