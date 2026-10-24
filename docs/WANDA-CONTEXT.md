# Wanda: personas, contexto y propuestas

Wanda usa el `PersonGraph` C++ existente. `archie-sentry export-json` (nunca con
`--all`) proyecta sus perfiles, etiquetas, relaciones declaradas e hipótesis; los rasgos
sensibles y sus notas salen redactados, para la vista y para el modelo. Las etiquetas
(`wanda.tag`) van sólo en `tags`. La proyección no convierte una hipótesis en relación
ni establece consentimiento. Las escrituras del CLI mantienen un lock del sistema
durante carga, modificación y guardado; las lecturas (`show`, `check`, `person`,
`mutual`, `path`, `export-json`) no lo toman, porque cada guardado es un rename
atómico: un lector con acceso de sólo lectura funciona y no espera a un escritor.

`archie-wanda` conecta esa autoridad con Vault, `PillarContext`, Cerebra y el runtime
de modelos existente. No mantiene una segunda base de personas ni otro Cortex.
Los mensajes completos quedan cifrados como `Restricted`, con caducidad configurable
de 1–720 horas (168 por defecto). Cada dispositivo tiene un token, fuente y dominio
fijos; sólo la configuración local puede asignar un chat a un proyecto laboral, y sólo
por su JID (`<número>@s.whatsapp.net`, `<id>@g.us`, `<id>@lid`; `@c.us` equivale a
`@s.whatsapp.net`). El título de un chat no es único y cualquier miembro de un grupo
puede cambiarlo, así que los colectores lo envían como `title:<nombre>` y nunca enruta:
ese evento se queda en el dominio de su fuente. Un reenvío del mismo mensaje no lo
reescribe ni lo vuelve "nuevo"; sólo actualiza la última vez que se vio la fuente.
Los productores no pueden enviar rutas, elegir un dominio ni declarar permisos.

## Ejecutar

Ejemplo de configuración (rutas relativas al archivo):

```toml
state_dir = "/var/lib/archie"
graph_binary = "/usr/bin/archie-sentry"
models_dir = "/etc/archie/models"
operator_token_file = "/var/lib/archie/secrets/wanda-owner.token"
quality_tier = 50
deep_quality_tier = 80
retention_hours = 168

[[domains]]
domain = "personal/owner"
graph_path = "/var/lib/archie/sentry/personal.tsv"

[[domains]]
domain = "work/studio/house"
graph_path = "/var/lib/archie/sentry/house.tsv"

[[sources]]
device_id = "ENROLLED_DEVICE_ID"
domain = "personal/owner"
source = "whatsapp_visible_dom"
token_file = "/var/lib/archie/secrets/wanda-browser.token"
allow_presumed_owner = true
[sources.chat_domains]
"120363042000000001@g.us" = "work/studio/house"

[core]
# Raíz de Archie con el diseño de ARCHIE_ROOT; sin ella, /etc/archie y /run/archie.
# root = "/var/tmp/archie"
focus = "wanda"
timeout_seconds = 60
```

### Razonamiento por Core

Con el bloque `[core]`, Wanda no llama al modelo: cada consulta es un `AiInfer` en el
socket del operador de `archied`, fijado al modelo que Cerebra admitió, con grado
`Restricted` y sin la memoria propia de Core (Wanda ya eligió el contexto). Core admite la
consulta con su perfil (`etc/inference.toml`), la despacha con un permiso de Policy,
el router vuelve a comprobar grado y localidad, y la respuesta queda sellada en el Vault
de Core con un recibo. La respuesta de `/v1/query` trae ese `receipt`.

- El proceso de Wanda corre como el dueño del perfil de Core, y los dominios de Wanda
  deben estar en `allowed_domains`. El prompt de Wanda llega a ~34 KiB con el contexto
  completo: subir `max_prompt_bytes` a 65536.
- El modelo debe existir con el mismo id en `models_dir` de Wanda y en el catálogo de Core:
  usar el mismo directorio (`/etc/archie/models`).
- La pausa de emergencia (`archie ai pause`) también detiene a Wanda.
- Sin Core en marcha, Wanda responde `core_unavailable` y no consulta al modelo por fuera.
- Sin el bloque `[core]`, Wanda llama al modelo local directamente y no queda recibo. Es
  el modo de diagnóstico y de las pruebas sin base de datos.
- El experto LNCM (estado 1) corre dentro del proceso de Wanda y no pasa por Core.

Usar rutas sin `..` después de resolver la ubicación del archivo; la validación
rechaza componentes que puedan escapar del directorio configurado. En producción
conviene escribir rutas absolutas. Los archivos de los dos grafos deben ser distintos.
Crear el grafo con el CLI existente bajo `ARCHIE_SENTRY_GRAPH`; el servidor nunca
sobrescribe ni inicializa un grafo faltante. Crear el token del productor con 32 bytes
de aleatoriedad del sistema, codificados en hex; no reutilizar el del operador.
El token del operador se genera una vez bajo el lock del Vault y no se imprime.

```sh
cargo build --manifest-path core/Cargo.toml -p archie-wanda
archie-wanda --config /etc/archie/wanda.toml --listen 127.0.0.1:8791 --proactive
```

La vista se sirve en `http://127.0.0.1:8791/`. Emparejarla con el token del operador;
la pestaña conserva el token sólo en `sessionStorage`. La voz usa:

```sh
export ARCHIE_WANDA_ENDPOINT=http://127.0.0.1:8791
export ARCHIE_WANDA_TOKEN_FILE=/var/lib/archie/secrets/wanda-owner.token
export ARCHIE_WANDA_DOMAIN=personal/owner
```

El tool `wanda_context` existe en la voz con feature `wanda`/`live`. Antes de leer
credenciales o hacer HTTP comprueba el endpoint y descriptor realmente elegidos
para esa sesión: inferencia local en loopback, proveedor coincidente y grado
`Restricted`. Una sesión cloud recibe una negativa sin información de Wanda.
El modelo de voz no puede modificar el dominio a través de argumentos del tool.

## API y presupuesto

| Ruta | Autoridad | Datos |
|---|---|---|
| `POST /v1/events` | Token de productor | Array del esquema `protocols/wanda-event.schema.json` |
| `POST /v1/query` | Operador | `{domain, question, deep}` |
| `GET /v1/view?domain=…` | Operador | Grafo, fuentes y propuestas del dominio |
| `GET /v1/domains` | Operador | Dominios configurados |

No cookies, CORS general ni tokens en URL. El servicio sólo admite bind loopback.
Un proxy TLS autenticado para productores remotos debe conservar un Host loopback
hacia el servicio; la vista/consulta privada conservan su validación de Origin.
La extensión usa permisos de host para su transporte; no se expone el API del
operador a otros orígenes. Límites: request 256 KiB, batch 32, texto 2000 caracteres,
identificadores 192 bytes, contexto 64 referencias y 32 KiB de contenido seleccionado.
La proyección completa admite 4096 nodos y 16384 relaciones; el contexto del modelo
selecciona como máximo 64 nodos, 128 relaciones y 24 rasgos por nodo. Esa selección
es limitada y no garantiza recuperar todas las personas relevantes de grafos grandes.

El modo `--proactive` comprueba contexto cada 30 s. Agrupa cambios mediante referencias
y fecha de modificación del grafo; repeticiones no disparan otra inferencia. Mantiene
`parallel_queries` inferencias en vuelo (1 por defecto; igualarlo a los slots de
llama-server), sin cola ilimitada: una consulta o edición espera su turno hasta 25 s y
luego recibe 503. Una consulta lee como mucho 48 mensajes nuevos antes de responder; el
ciclo proactivo lee el resto (480 por dominio y ciclo). El estado social se lee de memoria:
sólo el servicio lo escribe y cada guardado actualiza la copia. Los resultados se conservan en memoria del servicio; reiniciarlo no elimina
evidencias, pero requiere recalcular las propuestas. Una consulta que no produce
propuestas (modelo caído, salida inválida) no borra las últimas válidas, y el ciclo
proactivo reintenta esa misma evidencia con espera creciente (1 min, 2, 4… hasta 1 h).
El grafo que ve el modelo es el dueño y las personas más cercanas, recortado de una en
una hasta caber en 16 KiB; la instantánea citada vive tanto como puede vivir una
propuesta (24 h). Todo el trabajo con el Vault corre fuera de los hilos del runtime. `pending_events` es una cota del
lote reciente pendiente, no el número total de mensajes históricos.

`deep` exige el piso de calidad configurado en el router existente, no un tamaño de
modelo supuesto. Sólo se usan manifiestos realmente presentes, endpoints locales en
loopback y el modelo elegido por Cerebra. El transporte local ignora proxies de entorno.
Sin un modelo elegible se devuelve `no_model`, conservando el contexto en el equipo.
El lock del Vault se libera antes de inferencia; una llamada al modelo tiene 20 s de
presupuesto y el cliente de voz 30 s. Esto no establece un benchmark de latencia.

Cada propuesta exige citas admitidas, personas existentes, estado `inferred`, caducidad
y condición de revisión; una interpretación social exige alternativas. La validación
estructural no demuestra que el texto generado sea verdadero. No hay envío de mensajes,
compras, mutaciones de calendario ni admisión automática a REM/aprendizaje.

## Evidencia y trabajo pendiente

Pruebas locales: CTest 18/18; modelo de vista 11/11; collector navegador 4/4; pruebas
de voz sin SDK 55/55. Integración real del servicio con CLI C++ y navegador validó
emparejamiento, perfiles, filtros, aislamiento laboral y viewport de 390 px. Los ensayos
de inferencia usan un servidor sintético para verificar el recorrido, no pesos reales.

- [ ] Compilar Android con SDK 34/Gradle y probar una notificación real.
- [ ] Validar selectores y atribución en WhatsApp real; cobertura actual: chat visible
  y notificaciones, no historial completo ni mensajes de todos los chats.
- [ ] Integrar el productor real de IRIS/host para todos los monitores. `iris_semantic`
  es la entrada disponible; estos cambios no implementan captura de Windows.
- [ ] Selección/retrieval relevante en grafos grandes, resolución explícita de remitente
  a persona, correcciones y confirmaciones por voz usando las autoridades existentes.
- [ ] Fuente de ofertas vigente con precio/moneda/stock/URL/caducidad comprobables.
  Un chat que menciona $22.900 no acredita una promoción actual.
- [ ] Comparación explícita de agenda personal y laboral con autorización compuesta;
  una consulta por proyecto no absorbe contexto personal automáticamente.
- [ ] Evals con modelo real: preferencias, conflictos, abstención e hipótesis sociales;
  ninguna inferencia de intención o diagnóstico se presenta como certeza.
- [ ] Verificar interrupción de voz durante tools lentos, ASR/TTS y hardware real.
- [ ] Revisar el nuevo conjunto sobre main actualizado, integrar Quickshell y empaquetar
  servicio/voz en WSL con Claude/Xavi. La imagen ya subida a Sheriff no incluye esta rama.

Reproducir suites Rust configurando `WANDA_TEST_GRAPH_BINARY` al CLI compilado y
ejecutando `cargo test --manifest-path core/Cargo.toml -p archie-wanda`, y la integración
C++ explícita con `cargo test --manifest-path core/Cargo.toml -p archie-wanda --test graph -- --ignored`.
Para navegador: `NODE_PATH` con Playwright, Edge instalado y
`node --test display/wanda/tests/browser.cjs`. `WANDA_QA_DIR` elige el directorio de
capturas; los datos son fixtures y no se usa una sesión privada del usuario.

Ver collectors y sus límites en [capture-agents/WANDA.md](../capture-agents/WANDA.md).
