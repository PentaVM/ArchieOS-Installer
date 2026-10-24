# Prueba de voz y floor

Esta copia mantiene el ASR Nemotron, el floor y Piper existentes. Añade herramientas asíncronas cancelables, conserva el nombre detectado por cualquiera de los reconocedores y libera como máximo dos segmentos hacia reproducción. Las acciones siguen pasando por sus permisos originales.

## Abrir la copia preparada

En Windows, ejecuta `Start-VoiceTrial.ps1` desde la carpeta de entrega. Usa audífonos: di «Archie, dime una frase corta». Ctrl+C cierra esta sesión. La copia se instala en una carpeta propia y usa el servidor local Bonsai que ya esté disponible en `http://127.0.0.1:8081/v1`.

Para usar altavoces:

```powershell
.\Start-VoiceTrial.ps1 -Speakers
```

Con altavoces el micrófono se cierra mientras Archie habla: aún no hay AEC. Con audífonos queda abierto para probar correcciones. El lanzador no inicia servidores ni detiene otra sesión o entrenamiento. Esta prueba de conversación no conecta Stage ni permite acciones de escritorio.

Si Claude cambia el servidor local compatible con OpenAI, configura explícitamente la dirección y el modelo:

```powershell
.\Start-VoiceTrial.ps1 -Endpoint 'http://127.0.0.1:8081/v1' -Model 'bonsai'
```

La espera de una respuesta está limitada a 600 segundos. Ese techo permite probar el servidor CPU existente; no es una promesa de latencia interactiva. ASR/Piper se ejecutan sin CUDA, con afinidad de dos cores y prioridad reducida. No se modifica Wanda/LNCM.

En el smoke sintético, el GGUF de ASR avisó que no lleva tokenizer para reforzar el nombre y transcribió «Aquí» al oír «Archie». No se añadió esa palabra común como activador. Para una prueba individual sin depender de reconocer el nombre, usa explícitamente:

```powershell
.\Start-VoiceTrial.ps1 -Always
```

Ese modo responde a cualquier petición que oiga; úsalo solo para la prueba individual. El reconocimiento del nombre en una habitación real queda pendiente de verificar.

## Qué probar

1. Di «Archie, si mañana…», pausa y completa la cláusula. Debe conservar lo pendiente y evitar responder a una frase claramente incompleta.
2. Pide una explicación. Durante su voz di «ajá» o «sí». Un reconocimiento breve no debe cancelar toda la respuesta.
3. Con audífonos, interrumpe con «Archie, no, dame una frase más corta». Debe cancelar lo pendiente y atender la corrección; la historia solo conserva segmentos cuya reproducción terminó.
4. Haz una pregunta de seguimiento tras la respuesta, sin repetir el nombre. Pasada la ventana de seguimiento, vuelve a nombrar a Archie.
5. Si falla el servidor, comprueba que aparezca una alerta y que el loop vuelva a escuchar; no debe inventar una respuesta ni confirmar una herramienta sin resultado.

Los mensajes `timing` llevan turno, fase, tiempo monotónico desde la admisión y profundidad de segmentos liberados. `first_segment` mide liberación de texto, y `primer audio` mide síntesis del segmento; ninguno acredita la latencia que realmente oye una persona.

## Preparar otra copia desde este código

Compila en Linux con el SDK NeMo-Speech instalado. No hacen falta pesos para compilar o ejecutar los tests controlados:

```sh
cd core
CUDA_VISIBLE_DEVICES= CARGO_BUILD_JOBS=1 NEMO_SPEECH_PREFIX=/opt/archie/nemo-speech \
  cargo build --locked -p archie-voice --bin archie-voice --features speech,live,rvc
cd ..
sh scripts/voice/prepare-floor-trial.sh core/target/debug/archie-voice "$HOME/.local/share/archie-voice-floor-trial"
```

El preparador rechaza un destino existente. Si el worktree pertenece a Git de Windows, pasa `ARCHIE_VOICE_SOURCE_REV` con el SHA completo obtenido en Windows; Git de Linux no puede resolver ese `.git`. Los paths de modelos y venv se pueden cambiar con `ARCHIE_ASR_MODEL`, `ARCHIE_PIPER_ES`, `ARCHIE_PIPER_EN` y `ARCHIE_PIPER_PYTHON`. El archivo `SHA256SUMS` identifica el binario y bridge preparados.

## Límites conocidos

- El floor sigue siendo heurístico. Los estimadores aprendidos y la unificación con el floor de Cerebra requieren otra validación.
- El browser usa temporización de audio estimada; no hay ACK de reproducción física. La validación automatizada usa límites controlados; la prueba de habitación queda para el operador.
- Cancelar una herramienta aborta la espera y sus continuaciones; no revierte un efecto físico ya realizado. Si se interrumpe una operación Stage, se descarta su transporte para evitar reusar respuestas a medias y se requiere reiniciar la sesión de voz para reconectarlo.
- El segundo reconocedor opcional sigue siendo síncrono; la copia preparada usa un solo ASR. Una llamada nativa bloqueada de ASR/TTS o una llamada PulseAudio bloqueada todavía puede retrasar la parada.
- El proveedor puede acumular salida en el transporte, sujeto a su límite de respuesta; el loop solo procesa hasta 32 updates y un segmento por poll, con 16 KiB por delta y 64 KiB de texto por paso del modelo. Un exceso falla explícitamente.
- La API antigua `converse` mantiene compatibilidad con callbacks síncronos. Los dos modos interactivos del CLI usan la API asíncrona.

La arquitectura previa está documentada en `_dev/verification/integration/floor-voz-arquitectura-20261021.md`; los resultados de esta implementación están en `_dev/verification/integration/speech-hardening-results-20261021.md`.
