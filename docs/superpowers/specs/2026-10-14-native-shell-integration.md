# Integración nativa de la shell con ArchieOS

Solicitud vigente: conservar PR41 abierto, subir avances por etapas, auditar con el
contexto completo del propietario, integrar Core/Cortex/IRIS y render existente,
optimizar recursos, modo claro beta al final. **Nunca fusionar sin petición nueva.**
La instrucción directa autoriza ahora implementar la GUI; reemplaza la limitación
anterior de sólo auditar el trabajo de Xavi. No autoriza reemplazar autoridades.

## Arquitectura y alternativas

Se elige el host C++/Qt existente: el mismo `archie-display` ofrece modo shell,
un único viewport LumenField/LumenScene y el bridge tipado existente. Evita Chromium,
Python, capturas de respuestas externas y otro grafo/controlador de inferencia.
Alternativas: mantener WebEngine conserva el visor externo pero exige dos motores,
assets externos y una semántica de grafo distinta; un plugin Quickshell separado
requiere empaquetar ABI/plugin/compositor adicionales. La ruta elegida admite una
sesión Wayland y mantiene Observatory/EGLFS como recuperación.

"Incrustado/low level" se concreta en componente nativo del OS con lifecycle y
presupuesto existentes; gráficos y transporte viven en espacio de usuario. No se
introduce un driver de kernel para Qt, navegadores, IPC HTTP o inferencia.

Core/Cerebra decide; Policy autoriza; Stage ejecuta. La pantalla sólo consume
proyecciones minimizadas y presenta controles con contrato realmente disponible.
No solicita un lease falso, no llama proveedores de modelos con contexto privado,
no escribe Vault ni finge que una operación sin endpoint fue ejecutada.

## Datos y rutas

- Snapshot existente: status, modules, models, topology, metrics, source/freshness.
- CortexTrajectories ya existe en Core: transportar su DTO minimizado, limitado,
  sin referencias privadas Vault ni contenido de contexto. Providers revela
  metadatos de manifiestos, jamás valores de credenciales.
- IRIS/Cortex/voz/Stage ya publican WorkflowView en el Orb. Mostrar observed,
  phase, count, age y active; ausencia de señal no significa servicio inactivo ni
  servicio disponible. No inventar un nuevo broker.
- Ampliar proyecciones GET y Frame schema 1 aditivamente. Sin POST de ejecución
  en displayd, sin socket privilegiado ni membresía Core nueva para el renderer.
- LAN y jobs conservan procedencia y frescura independientes del daemon central.
- No hay endpoint operativo de gestor de archivos Vault, worker Forge, ni sesión
  gráfica para las aprobaciones de operador de Sentry. Esos controles quedan
  indisponibles con razón explícita; no se presenta simulación como integración.

## Render y compatibilidad

Reutilizar geometría, instancing, shaders, cámara, picking y eventos cinematográficos
del renderer nativo. Un controller alimenta un viewport activo. Preservar perfiles
field/crown/atlas/dense, overlays, comparación Antes y voz existente. Nuevos presets
son aditivos; no adoptar las eliminaciones del PR40.

Eliminar WebEngine del camino de arranque operativo. El visor externo puede quedar
documentado como experimento, separado de producción; no ejecutarlo como root ni
regrabar estado global para dibujar el escritorio.

El presupuesto del kernel/gobernador existente controla intervalos y efectos.
Intervalo 0 detiene timers. Oculto/minimizado no consume ticks de render; se mantiene
un reloj lento para frescura. Reduced/Quiet no gira la esfera ni hace viajes de
cámara decorativos. Solicitudes obsoletas no publican escenas de otra lens.

## UI honesta y dominios

Validar payloads completos antes de aplicarlos. Rechazar tipos/estados inválidos sin
mutar el último frame bueno. Mostrar silencio/edad y no un contador actual falso.
Modelos/proveedores/módulos/Cortex/workflows/jobs se dibujan desde fuentes reales.
Permisos/autoridad y canales sin contrato se muestran desconocidos/indisponibles.
Quitar copia/progreso/undo simulados del flujo operativo; muestras sólo con etiqueta
explícita. Personal/trabajo no se infieren de una selección visual ni mezclan datos.
Paleta/ratón/Enter comparten elegibilidad; controles cerrados no aceptan clicks.

## Tema, verificación y entrega

Modo oscuro existente primero. Modo claro beta al final mediante tokens semánticos
compartidos; no duplicar paneles ni agregar colores sin relación al tema. Mantener
marca y categorías separadas del contraste de texto.

Pruebas RED/GREEN de contratos, permisos read-only, frescura, UI sin WebEngine,
cinemáticas, límites, lifecycle y tema. Build C++/QML real y Frame backend con
proyección fixture explícita; fixtures no prueban GPU, modelos ni actuadores.
Registrar comandos, resultados, límites y pendientes en bitácora.

Implementación en checkout propio `codex/gui-backend-integration`, derivado de
eeeb68c. Subir a la rama del PR41 únicamente por fast-forward, sin force, sin merge
local ni de GitHub. Si Xavi publica cambios, inspeccionar y preservar antes de push.
No reconstruir/reemplazar el release WSL que Sheriff está probando.
