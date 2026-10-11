# Integración personal y laboral de ArchieOS

## Objetivo y autoridad

Archie observa y prepara trabajo con contexto del propietario mientras este usa el
equipo. Core conserva autoridad; los modelos asesoran, Policy autoriza y los
adaptadores existentes ejecutan. Las entregas y respuestas se presentan para
verificación y modificación. No se promete un porcentaje de ahorro sin medirlo.

La autorización de implementación procede de la petición del propietario de
integrar los pilares sin volver a solicitar permisos técnicos. No autoriza enviar
mensajes a contactos durante el desarrollo ni desplegar servicios desconocidos.

## Base comprobada

Worktree: ArchieOS-nvidia-unify, feat/unify, base 34bf7d4. Incluye Cerebra,
Vault, IRIS, Stage, puentes de aplicaciones y el supervisor archie-rem.
El puente Enterprise de archie-workstation conserva cuarentena y credenciales
efímeras. Su transporte remoto todavía no está implementado. El Sentry
operacional más reciente está en la copia Archie; no se mezclarán ramas enteras
durante este cambio. El REM Python se encuentra en archie-iris-rem.

## Diseño

1. Dominio explícito: `personal/<owner>` o `work/<organisation>/<project>`.
   Son identificadores opacos, no nombres ni contenido. Las referencias Vault
   del nuevo circuito quedan bajo ese prefijo; el prefijo completo y su separador
   impiden mezclar proyectos o vida personal. El prefijo ya forma parte del AAD
   del registro cifrado de Vault. Los registros históricos no se migran ni se
   reclasifican automáticamente.
2. Gateway verifica el conjunto completo antes de abrir ninguna entrada. Cerebra
   transmite el dominio de la observación al gateway y valida antes del ranking.
   Los consumidores históricos con otros namespaces mantienen compatibilidad;
   personal/ y work/ requieren dominio explícito. Esa ruta histórica no acredita
   aislamiento y debe migrarse explícitamente.
3. Un contexto cognitivo por tarea reúne referencias, no cargas sensoriales:
   Sentry (hechos del entorno), IRIS (estado de la aplicación), Wanda
   (hipótesis y preferencias de interacción), Vault (evidencia duradera), Forge
   (trabajo técnico) y resultados verificados. Las referencias de otro dominio
   se rechazan. Un modelo local se selecciona mediante el router existente,
   sin fijar Qwen/MiMo ni darles autoridad. El contexto es Restricted por defecto.
4. El bridge Enterprise marca sus entradas como laborales, mantiene cuarentena,
   y nunca exporta explícitamente contexto personal. El acceso al tablero no
   constituye autorización para cambiar dominios ni liberar cuarentena.
5. REM recibe únicamente trabajos verificados de un dominio que haya habilitado
   aprendizaje, en buffers separados. No convierte observación general en
   dataset ni usa un adapter de otro proyecto por defecto. El supervisor y sus
   garantías de borrado/interrupción se preservan.

## Recorrido objetivo y límites

Mensaje laboral -> ingreso en cuarentena -> Core admite tarea y dominio ->
Wanda recomienda interpretación/tono como hipótesis -> Vault recupera referencias
del proyecto -> Cerebra rutea inferencia -> Forge/puente CAD propone operaciones ->
Policy/Stage/ejecutor autorizan -> IRIS y adaptador verifican resultados -> Desk
presenta planos y borrador de respuesta -> propietario revisa -> salida por
adaptador autorizado -> memoria guarda recibo -> REM aprende solo si está habilitado.

No existe un transporte WhatsApp integrado comprobado, ni un recorder Stage/IRIS
que alimente el supervisor REM. Esos consumidores se documentan como pendientes,
no se reemplazan con simulaciones que reporten éxito. El scheduler debe imponer
presupuesto/preempción; entrenar durante juego no es el comportamiento REM.

## Validación

Probar rechazo antes de descifrado, aislamiento entre proyectos, referencias
canónicas, contexto sin texto en Debug, propagación a cápsula, conservación de
cuarentena y rechazo de exportación personal. Ejecutar suites Vault, Cerebra,
Workstation y REM serialmente; ninguna prueba enviará datos a servicios reales.
