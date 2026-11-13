# Etapa 4D — Recuperación del RPC sin reiniciar el agente

## Objetivo

Cuando se corta temporalmente el transporte entre GPUI (Windows), Herdr Core
y la máquina seleccionada (p. ej., JaviPi), **el proceso Gentle Shell/Pi debe
seguir ejecutándose en la misma terminal `javipi/tN`**. Al restablecerse el
transporte, GPUI debe recuperar **esa misma sesión**, sin `terminal.run`,
sin nuevo proceso, sin reinstalar nada en ARM64 y sin reejecutar comandos.

Esta etapa complementa 4B-P: 4B recupera los agentes tras cerrar y volver
a abrir GPUI; 4D gestiona una interrupción de transporte **mientras GPUI está
abierto**, incluso en medio de un turno.

## Implementación

- El `TerminalSession` existente ya reintenta conectar `hc connect` con
  la dirección estable. Al agotar su lote de reintentos, el backend RPC activa
  nuevamente `reconnect()` pasado un intervalo, sin ejecutar `terminal.run`.
- El backend RPC distingue **ConnectionLost** y **ConnectionRestored** del
  evento irreversible **Exited**. Un cierre definitivo de la terminal sigue
  siendo un cierre definitivo; no se crea un agente de reemplazo.
- En un corte, se invalidan los receptores de solicitudes RPC en vuelo.
  Como algunas solicitudes pudieron llegar a Pi, **no se reenvían jamás
  comandos mutantes** (prompt, bash, abort, set_model, respuesta de diálogo).
  Si el resultado del comando quedó indeterminado, el usuario debe observar
  el estado recuperado antes de decidir si es necesario repetirlo.
- Al reconectar, se descarta el fragmento JSONL anterior y se inicia un
  decodificador de trama nuevo. Los cuadros de repintado de pantalla no
  vuelven a disparar eventos UI, tool ni de conversación históricos.
- En un hilo separado del lector de terminal, se valida el canal reabierto
  con un `get_state` **de solo lectura**, repetible y correlacionado. Durante
  la validación se bloquea el envío normal de RPC; el lector puede procesar
  respuestas del probe. Si falla, se vuelve a conectar el **visor**.
- El identificador de generación evita aceptar una respuesta tardía de un
  visor anterior o de una pestaña ya cerrada.
- Cuando el probe termina correctamente, GPUI vuelve a leer `get_state`,
  modelos y `get_messages` del mismo agente. Si el turno estaba activo,
  consulta `get_state` periódicamente hasta que termine y recupera el
  historial final, incluyendo lo completado durante la desconexión.
- Si el backend está configurado como `local` nativo, no usa Herdr.

## Limitaciones deliberadas

- Se restaura el estado/historial; **no se garantiza reemitir exactamente
  cada delta de streaming** que ocurrió sin conexión. No es un registro
  transaccional de eventos, y la UI puede mostrar un resultado parcial hasta
  el siguiente snapshot o el fin del turno.
- No se almacena una cola offline de prompts ni de acciones. No existe
  deduplicación general de comandos en el proceso Pi remoto.
- Un proceso remoto realmente finalizado, una dirección borrada o una máquina
  que no vuelve a conectarse no pueden recuperarse sin intervención. El
  programa muestra el problema y no crea un proceso alternativo.
- La pestaña remota puede mostrar datos del historial mientras reconstruye
  mensajes en vuelo. Una extensión podría exigir interacción de UI nueva
  después de la recuperación, y puede requerir validación específica.
- La lógica usa `get_state.isStreaming` cuando el backend lo informa para
  distinguir que un turno terminó durante la caída.

## Pruebas automatizadas (en Windows)

Usá la rama `feature/herdr-stage4d-live-agent-reconnect`:

```powershell
cd D:\Programacion\gentle-shell-gpui-herdr-core
git fetch origin
git switch feature/herdr-stage4d-live-agent-reconnect
git pull --ff-only origin feature/herdr-stage4d-live-agent-reconnect

cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

## Pruebas funcionales (sin instalar nada en JaviPi)

1. `$env:GENTLE_SHELL_TERMINAL_BACKEND="herdr"` y
   `cargo run --locked -p gentle-shell-gpui`.
2. Usá **Machines → javipi → New agent**. Verificá que responde y que
   `hc ls` muestra la dirección `javipi/tN`; anotala.
3. Con el agente **ocioso**, cortá temporalmente la comunicación de red
   entre el PC y Herdr/JaviPi, sin apagar el servidor ni matar la terminal.
   Verificá el aviso de desconexión; al restaurar la red, observá el mensaje
   de recuperación y enviá otro prompt al mismo agente.
4. Repetí con una caída suficientemente larga como para agotar el primer
   lote de reconexiones. El nuevo reintento debe recuperar el visor.
5. Repetí durante una respuesta larga o herramienta no destructiva (p. ej.,
   espera/lectura de un archivo de prueba). GPUI debe recuperar el historial
   y, cuando termine, no quedar con el indicador de turno indefinidamente activo.
6. Compará `hc ls` antes y después: la terminal original conserva su ID;
   **no aparecen terminales extras** por reconectar.
7. Durante un corte, intentá un comando nuevo: debe fallar explícitamente,
   **sin enviarse después** al reestablecer la red.
8. Confirmá que un agente distinto (Windows Herdr o otra sesión JaviPi)
   continúa independiente. Cerrar **una pestaña** termina solamente su agente.
9. Confirmá que salir de GPUI y volver a abrirlo mantiene la recuperación
   4B-P, y que `GENTLE_SHELL_TERMINAL_BACKEND=local` no contacta Herdr.

**No hacer pruebas que apaguen JaviPi, eliminen sesiones, corten la energía
o maten Core:** el objetivo es simular pérdida temporal del transporte, no
una caída del proceso del agente.

## Estado de validación

Este documento describe una implementación de 4D en una rama de desarrollo.
Las pruebas sintéticas del parser y las garantías de generación están
agregadas. **No está validada todavía en Windows con Herdr/JaviPi ni se ha
confirmado una ejecución de Clippy o de los tests de workspace sobre esta
rama. No fusionar a main hasta completar la matriz.**
