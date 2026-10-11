# Relevo para continuar ArchieOS — 2026-10-11

> Actualización posterior del mismo día: el dueño ordenó trabajar solo en
> `PentaVM/ArchieOS-Nvidia`. Sentry del árbol histórico ya fue portado e integrado
> hasta `aaf2071`. La tabla y los pendientes inferiores registran el estado anterior;
> contrastarlos con `HANDOFF-2026-10-11-SENTRY-AUDITORIA.md` y
> `HANDOFF-2026-10-11-SPEECH-FLOOR.md`. No continuar features ni pushear al viejo repo.

## Leer primero

Este relevo distingue dos repositorios. No son checkouts intercambiables:

| Repositorio | Main y ubicación para continuar | Trabajo integrado |
|---|---|---|
| `PentaVM/ArchieOS` | `C:/Users/Penta/Desktop/Archie`, rama `main` | Sentry operativo Rust, consulta IoT HA, análisis pasivo TShark, módulo LAN portado y backlog de compatibilidad |
| `PentaVM/ArchieOS-Nvidia` | `C:/Users/Penta/.codex/worktrees/archie-nvidia-main-integration`, rama `main` | Dominios personal/laboral, contexto de pilares y bridge Enterprise, sobre LAN, REM, IRIS y Veil ya integrados por Claude |

La publicación final se comprueba con `git ls-remote origin refs/heads/main`.
No sustituir una arquitectura por la otra ni fusionar ambos repositorios completos
sin revisar sus diferencias. Este turno integra el trabajo en el main correspondiente
y porta únicamente el módulo LAN de Claude a ArchieOS.

## Contexto y preferencias del dueño

- Voz es la interfaz principal. Pantallas, segundo monitor, revisión y correcciones
  complementan esa interacción; Wanda es el modo de interpretación social existente.
- Conservar Core, Cortex/Cerebra, IRIS, Policy, leases, scheduler y autoridad determinista.
  No crear otro Cortex en Windows ni otro orquestador independiente en Home Assistant.
- Aprovechar hardware del host para inferencia, manteniendo en ArchieOS autoridad sobre
  modelos, adaptadores, tareas y procesos. El handoff original se leyó íntegramente.
- Mantener uso general del host: biblioteca Steam, aplicaciones y operaciones normales.
  Elden Ring y Rocket League eran ejemplos. Compatibilidad amplia exige mediciones;
  no se certifica «todo juego» ni equivalencia entre FP4 y rendimiento en gaming.
- Objetivo de hardware: Mac mini, DGX/RTX Spark, Strix Halo, servidores antiguos,
  RTX 2080, Titan y otras GPUs. Ver `TODO-COMPATIBILIDAD-MULTIPLATAFORMA.md` en ArchieOS.
- Prioridad actual: Sentry, IoT, routers, análisis de red defensivo, PCs/nodos y móviles,
  incluido iOS. IRIS tuvo trabajo y runs separados con Claude: no reescribirlo aquí.
- Integrar Sentry, Wanda, Vault, REM y Forge con los loops existentes. Separar vida
  personal de trabajo y conservar el bridge laboral de Santiago Rearte (`dfe413d`).
- Archie prepara artefactos y propuestas para revisar: CAD, contabilidad y mensajes.
  «Ahorrar 99 %» es una aspiración, no una medición realizada ni permiso para presentar
  hipótesis sociales, inferencias o acuses de recibo como hechos verificados.
- El dueño autorizó implementación, commits, merge y push; no volver a pedir la misma
  confirmación. Los permisos del producto y los protocolos de dispositivos se conservan.

## Qué quedó implementado

### ArchieOS: Sentry e IoT

Cinco commits operativos anteriores (`5012a14` a `34eaa19`) añaden intents tipados,
resolución de recursos emparejados, ejecución con postcondiciones medidas, recibos
durables, IPP y operador Linux. Se conservan: no hay aprobación desatendida nueva.

`0fabaff` añade `archie sentry-home-status <resource> --json`: solo GET de una entidad
Home Assistant configurada por Core. Sensores y trackers van en `telemetry_resources`,
fuera del registro actuable. Mantiene timestamp de la fuente, unidades y evidencia
estable; descarta atributos arbitrarios y credenciales. HA cache no prueba frescura.
El lector privado de tokens sigue siendo Linux; no se habilitó en Windows.

El mismo commit añade `archie sentry-network-analyze --scope ... --input ...
--baseline ... --json`. Importa metadatos mDNS IPv4 de TShark con límites y reutiliza
el clasificador/delta existente. No captura, escanea, empareja ni concede autoridad.
Descarta nombres originales. Ver `SENTRY-HOGAR-RED-VOZ.md` para formato y comandos.

### LAN de Claude: complementario, no reemplazado

Commit fuente `3e89187`, merged por PR #26 en Nvidia (`630ea77`) y portado a ArchieOS
como `e93821f`. `archie-lan scan/watch` pregunta por SSDP y mDNS, lee descripciones
UPnP solo del host anunciante y consulta estado Roku. No envía controles del dispositivo.
Tiene 18 pruebas de parsers/clasificación. El commit de Claude reporta router y Roku
hallados en una red real; ese run no se reejecutó durante esta consolidación.

La diferencia importa: `archie-lan` envía preguntas de descubrimiento; el importador
TShark analiza un archivo de forma pasiva. Sus clasificaciones son heurísticas,
no identidad criptográfica ni pairing. El inventario LAN aún no entra al world graph.

Revisión pendiente concreta del módulo LAN:

1. `modules/sentry/lan.toml` nombra `/usr/bin/archie-lan`, pero `distro/archiso/build.sh`
   aún no lo compila/instala ni lo acepta en su lista de binarios. Falta provisionar
   el directorio de salida y probar el worker instalado con sus permisos reales.
2. HTTP aplica timeout por operación de socket, no deadline absoluto de toda la
   respuesta. Validar también la estructura `LanUrl` en el límite de I/O: sus campos
   públicos permiten construirla sin usar el parser, aunque los llamadores actuales
   sí pasan por ese parser.
3. Acotar acumulación de respuestas SSDP, hosts/servicios mDNS y duración agregada
   de consultas de descripciones. Un intervalo de escucha no es un límite de memoria.
4. Inventario persistente: retención, permisos de archivo, estado parcial, backoff y
   alertas; integrar con Core/Sentry sin atribuir dueños a teléfonos por su presencia LAN.

Estos puntos son revisión de código y trabajo pendiente; no se presentan como
vulnerabilidades explotadas, corregidas o cubiertas por las 18 pruebas existentes.

### Nvidia: memoria personal/laboral y pilares

`da3eb6f`, merged en `18613ba`, incorpora:

- `ContextDomain`: `personal/<owner>` o `work/<org>/<project>` con segmentos validados.
- Gateway de Vault comprueba todas las referencias antes de ranking o descifrado;
  rechaza referencias personal/work sin dominio y cruces de dominio. El recibo guarda
  la etiqueta de dominio, no los contenidos de memoria.
- `PillarContext` reúne referencias de Sentry/Wanda/Vault/IRIS/Forge/resultado verificado
  para el sequencer existente. No autentica productores ni habilita herramientas.
- Enterprise y BoardAPI separan proyectos, bloquean exportación de tareas personales
  aunque se etiqueten public y preservan cuarentena. Metadatos externos no pueden
  sobrescribir las etiquetas confiables. El antiguo stub de envío ya no declara entrega.

Se preservan los avances de Claude: `d49bc92` (reasoning/REM), `3e89187` (LAN),
`f925944` (Veil) y sus merges. Veil sigue exportado en Vault junto a ContextDomain.
Ver `PERSONAL-WORK-AGENT.md`; esa capa aún no es el flujo completo de eventos reales.

## Qué sigue, en orden

1. Probar y empaquetar `archie-lan`, cerrar sus límites de recursos y unir el inventario
   autenticado con Sentry. Añadir observaciones HA sin inventar frescura del dispositivo.
2. Router: elegir adaptador según firmware y acceso reales; DHCP/vecinos/Wi-Fi/contadores,
   configuración y logs. Captura mediante dumpcap/TShark en interfaz autorizada,
   puerto espejo/TAP o gateway, con cuotas. Un PC conmutado no ve toda la LAN.
3. Blue team: baselines, DNS/flujos, correlación con endpoints, alertas explicables,
   respuesta reversible y verificación. Observación de red no significa malware.
4. Productores Sentry→PillarContext→Cerebra→Forge→resultado→Vault/REM: todavía faltan
   conectores y controladores reales. Reutilizar `dispatch::run_pass`, reconciliación,
   fences y los contratos de memoria existentes; evitar otro loop paralelo.
5. Voz: VAD/wake word/STT, intención tipada, respuesta TTS, detener/cancelar/tomar
   control. Reconocimiento del hablante, presencia y autorización son distintos.
6. Biometría: el gate C++ ya verifica dueño, lease, sensor, desafío, frescura y liveness.
   Falta lector/worker real de voz y otras modalidades, templates en Vault, autenticidad
   del worker, calibración y pruebas de replay/síntesis. No fingirlo con un booleano.
7. Nodos y móviles: Fabric autenticado; Android existente; app compañera iOS con APIs
   y permisos de Apple. No prometer control arbitrario de otras apps ni daemon perpetuo.
8. Benchmarks reproducibles: HA p50/p95/p99, importación de red, latencia de alertas,
   falsos positivos, voz fin de frase→respuesta, cancelación, CPU/RAM/GPU y frame time
   de juegos concurrentes; TTFT frío/caliente y contexto por modelo/backend/hardware.

## Evidencia y límites del entorno

Windows y Rust 1.98.0. Antes de la integración LAN, Core/CLI/Sentry: 178 aprobadas,
0 fallos, 27 omitidas (fixture PostgreSQL y otros requisitos explícitos).
LAN portado: 18 aprobadas. Contexto sobre Nvidia main actualizado: 179 aprobadas,
0 fallos, 3 omitidas; incluye Vault/Veil, Cerebra, Workstation, REM, LAN y Model Runtime.
La bitácora de cada repositorio recoge los resultados finales después del merge.

El gateway local responde HTTP de lectura, pero no se identificó firmware/API ni
se autenticó administración. TShark/dumpcap no se encontraron en PATH. No se cambió
el router, activó contención, desplegó imagen ni ejecutó un benchmark de gaming.
No se certificó MSRV, ISO, Linux operator session ni dispositivos reales con estos tests.

## Higiene para continuar

- Los worktrees fuente permanecen; no borrar ni reaplicar diffs antiguos a ciegas.
- `ArchieOS-nvidia-unify` conserva una modificación antigua de Cargo.lock relacionada
  con dependencias de Claude que ya están en upstream. No reemplazar el lock de main
  por ese archivo ni tomarlo como una función nueva.
- Adjuntos locales, deliveries, snapshots y tmp de la carpeta Archie se conservaron
  localmente. No son commits de código y no se suben masivamente con `git add .`.
- No tocar el diff antiguo de `ArchieOS-cortex`: la bitácora histórica explica que
  fue integrado/corregido posteriormente. No revertir trabajo de otros agentes.
- Antes de cada push: fetch, comprobar avance remoto, merge normal, pruebas relevantes
  y comparar el hash remoto. No usar force push para resolver una carrera con Claude.
