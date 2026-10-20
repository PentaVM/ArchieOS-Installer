# Tabla de movimiento

Solo `transform` y `opacity`; nunca blur ni sombras animadas. Perfiles: **Full**, **Balanced**, **Quiet** (solo opacity, ≤120 ms), **Movimiento reducido** (cero movimiento espacial, estado instantáneo, glifo y palabra intactos). Lo decide el ajuste de la persona; sin ajuste, la preferencia del sistema «reducir movimiento» fuerza Movimiento reducido. Un pulso de éxito solo ocurre tras una confirmación real. Los eventos marcados **Diseño** aún no tienen disparo real.

| Evento | Qué se mueve | Full | Balanced | Quiet | Movimiento reducido | Curva | Disparo |
|---|---|---|---|---|---|---|---|
| **Ventanas** | | | | | | | |
| Abrir ventana | transform: scale(.96→1) · opacity 0→1, desde el icono del dock | 220 ms | 160 ms | 100 ms (opacity) | instantánea | Salida | Real |
| Cerrar ventana | transform: scale(1→.97) · opacity 1→0 | 160 ms | 120 ms | 100 ms (opacity) | instantánea | Entrada | Real |
| Minimizar ventana | transform: translate + scale hacia el icono del dock · opacity | 280 ms | 200 ms | 100 ms (opacity) | desaparece al instante; el punto del dock aparece ya | Estándar | Real |
| Cambio de escritorio | transform: translateX de las ventanas (±40 px) · opacity | 320 ms | 240 ms | 120 ms (fundido) | cambio inmediato; el indicador del escritorio se actualiza ya | Estándar | Real |
| Cambio Orbe ↔ espacio de trabajo | solo las ventanas: saliente opacity→0 + scale(.98); entrante opacity 0→1 + scale(1.02→1). El Orbe no se anima ni se toca | 320 ms | 240 ms | 120 ms (fundido) | cambio inmediato; el chip del espacio muestra el nuevo estado | Estándar | Real |
| **Navegación y selección** | | | | | | | |
| Paleta / búsqueda (abrir y cerrar) | transform: translateY(-8 px→0) · opacity | 180 ms · 120 ms al cerrar | 140 ms · 100 ms | 100 ms (opacity) | instantánea | Salida / Entrada | Real |
| Entrar a una carpeta | el contenido: translateX(12 px→0) · opacity; las migas cambian sin animación | 200 ms | 150 ms | 100 ms (opacity) | instantánea; migas actualizadas ya | Salida | Real |
| Selección de fila o ficha | una capa de selección: opacity 0→1. Nunca se desplaza nada | 100 ms | 100 ms | 100 ms | instantánea (el foco visible sigue igual) | Estándar | Real |
| Conmutador lista / cuadrícula (ya existe) | crossfade del contenido: opacity; el segmento activo mueve su fondo con translateX | 200 ms | 150 ms | 100 ms (opacity) | instantáneo | Estándar | Real |
| Escalado del dock (ya existe) | transform: scale(1→1.12) al pasar el cursor; el punto de «abierta» no se mueve | 160 ms | 120 ms | sin escalado: solo opacity del resalte, 100 ms | sin escalado; el resalte aparece ya | Salida | Real |
| **Avisos y estado** | | | | | | | |
| Notificación aparece | transform: translateY(-12 px→0) · opacity | 240 ms | 180 ms | 100 ms (opacity) | aparece ya; la palabra y el icono de estado intactos | Salida | Real |
| Notificación se agrupa | las tarjetas colapsan: translateY + scale(.98) hacia la pila; el contador cambia sin animación | 240 ms | 180 ms | 100 ms (opacity) | la pila se muestra ya con su contador | Estándar | Real |
| Notificación se retira | transform: translateX(0→24 px) · opacity 1→0 | 200 ms | 150 ms | 100 ms (opacity) | desaparece ya | Entrada | Real |
| Panel de control / Actividad (abrir y cerrar) | transform: translateY(-8 px→0) · opacity | 200 ms | 150 ms | 100 ms (opacity) | instantáneo | Salida / Entrada | Real |
| Cambio de estado de un dispositivo | pulso ÚNICO: un anillo ya dibujado hace scale(1→1.25) y opacity .6→0. Solo tras confirmación real del bridge; «sin lectura» o error nunca pulsan como éxito | 600 ms | 400 ms | pulso solo de opacity, 120 ms | sin pulso: el chip cambia ya; glifo y palabra intactos | Salida | Real |
| Enlace caído | el chip cambia por fundido (opacity); sin pulso ni parpadeo repetido | 160 ms | 120 ms | 100 ms (opacity) | cambio inmediato | Estándar | Real |
| Enlace recuperado | fundido del chip + un pulso único; nunca un éxito falso: se muestra «en vivo» solo con dato real | 160 ms + pulso 400 ms | 120 ms + pulso 300 ms | 100 ms (opacity) | cambio inmediato, sin pulso | Estándar | Real |
| **Disparo real aún inexistente · se diseñan, rotulados «Diseño»** | | | | | | | |
| Copia con progreso | la barra avanza con transform: scaleX (origen izquierdo), nunca con width; el porcentaje y la palabra cambian sin animación | continuo, sin curva decorativa | continuo | sin animación de barra: solo el valor | sin animación: valor y palabra | Lineal | Diseño |
| Diálogo de autorización | transform: scale(.98→1) · opacity; el foco entra en «Rechazar» sin animación | 200 ms | 150 ms | 100 ms (opacity) | instantáneo; foco igual en «Rechazar» | Salida | Diseño |
| Etapas de Forge | el marcador de etapa activa se desplaza con translateX; sin progreso inventado | 240 ms | 180 ms | 100 ms (opacity) | el marcador salta ya a la etapa | Estándar | Diseño |

```css
:root                    { --dur-open: 220ms; --dur-close: 160ms; --dur-pulse: 600ms; --ease-out: cubic-bezier(.16,1,.3,1); }
[data-motion="balanced"]  { --dur-open: 160ms; --dur-close: 120ms; --dur-pulse: 400ms; }
[data-motion="quiet"]     { --dur-open: 100ms; --dur-close: 100ms; --dur-pulse: 120ms; }  /* solo opacity */
[data-motion="reduced"],
@media (prefers-reduced-motion: reduce) { :root { --dur-open: 0ms; --dur-close: 0ms; --dur-pulse: 0ms; } }
```
