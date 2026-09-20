# ArchieOS: Workstation Enterprise Bridge

El módulo **Workstation Enterprise Bridge** (ubicado en `core/crates/archie-workstation`) es el componente de integración diseñado para conectar de manera segura instancias de ArchieOS con estaciones de trabajo corporativas, plataformas de gestión (tipo Jira, Trello) y clientes externos, bajo los estrictos principios arquitectónicos de **Zero-Trust** y **Privacy Spine** del sistema operativo.

## Propósito del Módulo

En ArchieOS, las estaciones de trabajo externas y sus APIs son consideradas recursos intercambiables y completamente carentes de autoridad sobre el kernel. El Enterprise Bridge actúa como el "guardián fronterizo" bidireccional, garantizando que el flujo de tareas y credenciales respete absolutamente las invariantes de seguridad de la red.

## Componentes y Arquitectura de Seguridad

### 1. Barrera de Autenticación de Borde (Strict Boundary Auth)
Todas las solicitudes hacia la API del Kanban Board (gestor de tareas externo) deben superar un middleware de Axum (`auth_guard`).
- **Autenticación sin Side-Channels**: La validación de los tokens `Bearer` entrantes se realiza procesando un hash SHA-256 contrastado de forma constante (constant-time verification) utilizando la librería `ring`.
- **Tráfico Ciego**: Se rechaza automáticamente cualquier solicitud anónima o con credenciales inválidas sin revelar información interna.

### 2. Ingress Privacy Spine (Asignación Forzosa y Cuarentena)
Cuando una carga útil (payload) ingresa al sistema a través del endpoint de creación de tareas (`POST /tasks`), el Enterprise Bridge intercepta el evento para aplicar saneamiento de seguridad:
- **Zero-Authority External**: Un sistema externo no tiene autoridad para clasificar tareas como públicas (`PrivacyGrade::Public`).
- **Cuarentena Obligatoria**: Toda tarea entrante se degrada automáticamente a `PrivacyGrade::Restricted` y es etiquetada con un flag de cuarentena (`"archie.quarantine": "true"`).
- **Anti-Privilege Escalation**: Si un atacante o sistema mal configurado intenta ingresar una tarea solicitando una etiqueta `Public`, la petición es mitigada, forzada al nivel restringido y registrada con una etiqueta de auditoría (`audit.escalation_attempt`).
- **Cerebra Router**: Las tareas permanecen en estado bloqueado o de cuarentena a la espera de que `Cerebra` (el motor de enrutamiento y cerebro principal del OS) las procese, analice y reclasifique si procede.

### 3. Egress Interlock (Filtro de Egreso)
El puente de salida responsable de sincronizar el estado del Grafo de Tareas del OS con los paneles corporativos remotos (`EnterpriseAdapter::prepare_for_export`) aplica un blindaje simétrico:
- Nunca exporta una tarea etiquetada en **cuarentena**.
- Solo emite datos basándose en el `PrivacyGrade` permitido de la conexión (por defecto, limitando los datos confidenciales `Sensitive`, `Restricted` y `Secret` y sólo abriendo canales para `Public` o `Internal`).
- Las tareas no autorizadas se excluyen silenciosa y limpiamente del lote de sincronización a través del mapeo seguro.

### 4. Zero-Secret Leaks (Vault Brokering)
El proceso de envío de datos al exterior y autenticación saliente se gestiona a través del sistema criptográfico del Kernel (`Vault`).
- En lugar de mantener tokens corporativos en el estado de memoria del Board, el Enterprise Bridge actúa como un *broker*. Solicita al `Vault` una credencial sellada en el momento de la ejecución.
- La credencial en memoria se expone sólo por los milisegundos necesarios para construir la llamada de red (`sync_to_remote`) y se destruye a nivel atómico en el hardware al salir de su ámbito de memoria (`Zeroizing`).

## Resumen del Flujo de un End-User Corporativo
1. **Ingreso:** Un webhook corporativo crea un ticket. Entra a `board_api.rs`, se despoja de permisos y entra a cuarentena.
2. **Procesamiento:** ArchieOS (Cerebra) extrae la tarea y autoriza la ejecución en la capa fabric/cloud si es seguro.
3. **Egreso:** Al finalizar o actualizar el progreso, el worker actualiza el DAG. El Bridge exporta el nuevo estado únicamente si la tarea ya no es confidencial ni está en cuarentena.

---
**Rutas Críticas del Repositorio:**
- `core/crates/archie-workstation/src/board_api.rs` (Handlers de Ingress/Axum Router).
- `core/crates/archie-workstation/src/enterprise_bridge.rs` (Exportador y reglas de Egress).
