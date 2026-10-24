# Wanda: personas, contexto y recomendaciones

## Objetivo y alcance

Wanda comprende el contexto social del propietario: pareja, familia, amigos y trabajo;
recuerda preferencias y compromisos útiles, explica interpretaciones y propone respuestas
y acciones. Voz es la interacción principal. Un mapa de personas, sus relaciones y
tarjetas de contexto permite inspeccionar y corregir la memoria. La solicitud del usuario
autoriza ejecutar este trabajo; no se detiene entre diseño, plan y desarrollo para pedir
de nuevo permiso. No se modifica el árbol activo de Kimi, la captura de IRIS, el Orbe,
la GUI global de Xavi ni el empaquetado WSL de Claude.

El trabajo se basa en `main` ac9c1d2. Se incorporan únicamente las piezas Wanda de
f890b32/6a977c4; los adaptadores y el follower de Sentry siguen siendo responsabilidad de
Kimi. Los cambios a persistencia compartida corrigen concurrencia sin cambiar autoridad.

## Arquitectura elegida

Se amplía `sentry::store::PersonGraph`/`GraphStore`, `Profile`, `RelationGraph` y el modo
Wanda ya existentes. No hay otra base de personas ni otro Cortex. La CLI C++ añade una
proyección JSON con versión y mantiene los mismos filtros y reglas de admisión. Una
capa Rust `archie-wanda` consume esa proyección, guarda evidencias en Vault y produce
recomendaciones mediante `Routed`, el registro y transportes de modelos existentes.

Alternativas descartadas: un grafo Rust paralelo duplicaría las reglas de identidad;
un asistente cloud autónomo exportaría contexto y rompería la autoridad existente.
La proyección del grafo y el razonamiento local pueden crecer de forma independiente.

## Dominios, etiquetas y hechos

Cada dominio `personal/<owner>` o `work/<organisation>/<project>` tiene una ruta de
GraphStore y un espacio Vault configurados por el propietario. Las rutas se resuelven
desde configuración confiable; un mensaje entrante no puede escoger otra ruta o dominio.
Los mismos identificadores de contacto en distintos dominios no fusionan sus datos.

Etiquetas de perfil usan los traits existentes; las etiquetas declaradas e inferidas
conservan su basis. Partner y Manager, si se añaden como relaciones, son tipos explícitos,
con las reglas de declaración existentes; no se disfrazan de Household o Employer.
Las hipótesis nunca establecen alcance, identidad ni derechos de un nodo.

La proyección JSON contiene `schema_version`, `owner_id`, `nodes`, `edges`:
nodes: `id,label,kind,degree,path_to_owner,tags,traits`;
traits: `key,value,basis,sensitivity,source_note,redacted`;
edges: `from_id,to_id,kind,basis,declarer_id,source_note,standing`.
Valores sensibles se ocultan por defecto. El receptor exige versión, IDs únicos,
extremos existentes y límites antes de usar el resultado. Timestamps desconocidos no
se sustituyen por una fecha inventada.

## Entrada viva

Un canal Wanda de contenido separado conserva intacto `capture-event.schema.json` y el
canal de metadatos de Sentry. Eventos v1: `event_id,device_id,actor,author,source,chat_id,
sender,source_message_id,sent_at_ms,observed_at_ms,text,media_omitted,coverage`.
Domain y permisos provienen del dispositivo/token configurado, nunca del texto.
Límites: request 256 KiB, batch 32, texto 2000 caracteres, IDs 192 bytes, cola 256 eventos,
evidencias por consulta 64. Los eventos de otro actor o fuente deshabilitada se rechazan
antes de persistir; `presumed_owner` se admite sólo si lo autoriza la fuente configurada.

Android usa MessagingStyle de NotificationCompat y conserva el chat y el remitente por
separado; previews ausentes o censurados se indican como cobertura incompleta. WhatsApp
Web observa sólo mensajes renderizados del chat visible mediante un content script;
no captura drafts ni contenteditable ni mantiene una sesión web independiente. El mensaje
conserva dirección, autor, tiempo y procedencia; las reposiciones se deduplican.
Una entrada `iris_semantic` admite resultados textuales del productor existente y marca
monitor/ventana cuando están disponibles. Wanda no crea una captura de pantalla rival:
el host completo y todos los monitores dependen del productor IRIS/host correspondiente.

El servidor escucha loopback por defecto. Clientes remotos usan un proxy TLS del
propietario, tokens por dispositivo y fuente, revocación y dominio fijado en configuración.
La API no autoriza herramientas ni envía mensajes a terceros.

## Memoria y concurrencia

Las evidencias se cifran con Vault en grado Restricted. Datos semánticos derivados y
preferencias del grafo conservan referencia a la evidencia y su standing. El texto vivo
es acotado; no se envía automáticamente a entrenamiento REM ni se conserva vídeo continuo.
Las referencias generadas entran en PillarContext, con dominio y tarea, en vez de descartarse.

Las operaciones de GraphStore y del Vault compartido deben cubrir lectura, modificación
y escritura con exclusión entre procesos. Las claves se crean una sola vez bajo ese lock;
un proceso que termina libera su lock. Los límites de lectura se aplican antes de cargar
el archivo completo. Un evento duplicado no vuelve a generar un hecho ni una recomendación.
Los checkpoints se guardan después de la evidencia; el replay recupera una caída parcial.

## Recomendaciones y razonamiento

Routed y el registro existente eligen los modelos elegibles. El contexto Restricted es
local por las reglas actuales, y una preferencia de calidad elevada se expresa a través
del mismo router, sin hardcodear una marca/tamaño de modelo ni registrar disponibilidad
inventada. La falta de modelo es visible y no genera consejos ficticios.

La entrada del modelo contiene sólo el dominio solicitado, evidencias citables y perfiles
pertinentes, con límite de 64 referencias. El resultado JSON se valida: toda recomendación
debe citar evidencias admitidas, distinguir conocimiento de interpretación, incluir
alternativas cuando interpreta intención, una condición de revisión y caducidad. Las
salidas no cambian automáticamente relaciones, calendario, permisos ni entrenamiento.

Casos de aceptación:
- Regalo: preferencia declarada por tulipanes + oferta vigente con merchant URL, producto,
  moneda, precio, verificación y expiración. 22.900 CLP es un fixture, jamás una oferta real
  inventada. Sin oferta actual, se recomienda el tipo de regalo y se explicita que falta
  consultar precio.
- Trabajo/pareja: dos compromisos con intervalos o plazo explícitos producen un conflicto
  explicado y una sugerencia de ajuste. El sistema no supone que salir impida trabajar.
- Interés social: señales observadas pueden originar una interpretación tentativa, con
  alternativas y fuentes; no se convierte contacto físico, amabilidad o co-presencia en
  prueba de atracción, consentimiento o un diagnóstico psicológico.
- Comunicación: respuestas propuestas consideran relación, preferencias y compromisos;
  se presentan para revisión y no se envían por este subsistema.

Los datos de una oferta no verificada o vencida no se presentan como precio actual. Las
interpretaciones sociales no son porcentajes calibrados ni diagnósticos clínicos.

## Voz y vista

La herramienta Wanda devuelve propuestas y evidencia. Los detalles Restricted sólo
pueden entrar en una sesión de voz local elegible; una sesión cloud recibe una indicación
sin datos privados o utiliza el contrato de respuesta externa de la voz sin reenviarlos
al modelo cloud. Se mantienen interrupción, presupuesto y recibos del floor existentes.

Una superficie Wanda propia muestra personas, grupos, etiquetas, caminos al propietario,
relaciones confirmadas, hipótesis diferenciadas, preferencias, fuentes y recomendaciones.
Puede alternar mapa/lista, buscar y filtrar por dominio y etiquetas. Sin datos reales
configurados muestra un estado vacío; fixtures de demostración quedan identificados.
Se entrega un DTO/superficie para la GUI de Xavi sin modificar su navegación global.

## Pruebas y límites de la entrega

Pruebas sobre almacenamiento real temporal y procesos concurrentes; export C++ real;
API sobre loopback; modelo con servidor compatible de prueba para validar transporte,
router, citas y rechazo de datos cruzados; fixtures DOM sanitizados y parsers de Android.
Se comprueban entradas hostiles, replay, corrupción, límites, ofertas caducadas y fuga de
contexto por voz. Fixtures no prueban exactitud de inferencia, WhatsApp real, pantalla
completa, hardware ni latencia. La entrega distingue esas verificaciones pendientes.
