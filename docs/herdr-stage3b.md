# Etapa 3B: reconexión de terminales Herdr

Esta etapa mantiene una única dirección de terminal durante toda la vida del
panel GPUI. Ante un corte de hc connect, vuelve a negociar hello/framed,
terminal.attach y recupera la pantalla actual de esa misma dirección. Nunca
vuelve a ejecutar terminal.run durante una reconexión.

## Cambios

- El lector y escritor del protocolo corren fuera del hilo gráfico GPUI.
- Cada canal conectado tiene una generación: créditos pendientes de canales
  anteriores no pueden acreditarse al canal nuevo.
- La conexión activa se marca como caída al recibir un error de lectura,
  un error de escritura o un CLOSE no definitivo.
- Las causas exited y killed se consideran finalización real: no reconectar.
  Las pérdidas transitorias de un relé o un programa que Herdr mantiene vivo
  son recuperables (sujetas a disponibilidad del core).
- Reintentos: cuatro esperas de 1, 2, 4 y 8 segundos, con cinco intentos
  de conexión por ciclo. Después aparece el botón Reconnect Herdr.
- Pulsar Reconnect Herdr reintenta la misma dirección; se conserva el panel.
- Cerrar el panel cancela el trabajo y termina el proceso de transporte
  hc connect, no el proceso de terminal de Herdr.
- Mientras está desconectado, el teclado no se encola ni reproduce al
  recuperar la conexión. Tamaño y foco sí mantienen su último valor.
- Al restablecer el canal, el procesador ANSI se reinicia y se limpia
  la pantalla antigua antes del repaint completo enviado por Herdr.

## Windows: compilación

Partir de la rama de 3A y descargar la siguiente:

    git fetch https://github.com/javijec/gentle-shell-gpui-herdr-core.git feature/herdr-stage3b-reconnect
    git switch -c feature/herdr-stage3b-reconnect FETCH_HEAD

O, con un remoto fork configurado:

    git fetch fork
    git switch --track fork/feature/herdr-stage3b-reconnect

Después compilar:

    cargo test --locked -p herdr-client
    cargo test --locked -p gentle-shell-gpui
    cargo clippy --workspace --all-targets --locked -- -D warnings
    $env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
    cargo run --locked -p gentle-shell-gpui

## Comprobaciones funcionales

1. Abrir UN panel GPUI con backend Herdr. Anotar la dirección tN con hc ls
   en otra consola. Ejecutar un comando interactivo y comprobar pantalla.
2. Encontrar solo el proceso de relé del panel (hc connect). Por ejemplo:

       Get-CimInstance Win32_Process | Where-Object {
           $_.Name -eq 'hc.exe' -and $_.CommandLine -match ' connect'
       } | Select-Object ProcessId, ParentProcessId, CommandLine

   Con un solo panel de prueba abierto, identificar su hc connect concreto
   y detener SOLO ese PID (nunca el daemon ni otros procesos):

       Stop-Process -Id <PID_DEL_RELE> -Force

3. GPUI debe mostrar temporalmente el error y la reconexión, pero debe
   recuperar la misma tN cuando un nuevo hc connect se pueda abrir.
   No se crea otra dirección tN. El cursor y el contenido deben repintarse.
   Los comandos que se escriban DURANTE el corte no deben ejecutarse después.
4. Comprobar hc ls: el proceso administrado por Herdr sigue vivo.
5. Hacer otra interrupción durante un TUI, si está disponible, y comprobar
   pantalla, cursor, teclado y redimensionamiento.
6. Si el enlace permanece inaccesible durante cinco intentos, debe quedar
   el botón Reconnect Herdr. Al restablecer la disponibilidad, pulsarlo debe
   recuperar la misma terminal sin crear otra.
7. Confirmar que salir de la shell realmente terminada muestra Process exited
   y no un bucle de reconexiones.
8. Cerrar GPUI y volver a abrirlo: verificar la restauración de 3A.

La prueba 6 requiere provocar un corte REAL de conexión o indisponibilidad del
core durante más tiempo. No mates la terminal de Herdr para simularlo: eso
prueba la terminación, no una caída de red.

## Pendiente

- La reconexión no puede finalizar un bloqueo indefinido dentro del handshake
  hc connect: falta un timeout de handshake explícito.
- No hay una prueba automatizada de integración con un Herdr Core real:
  se requieren validaciones manuales de Windows, Linux y macOS.
- No hay selector gráfico de máquinas (etapa 3C), ni estado de presencia
  persistente para una máquina remota offline.
- Los paneles previamente abiertos siguen ejecutando el backend optado por
  el usuario; no se migran dinámicamente entre un PTY local y Herdr.

Protocolo: https://github.com/herdrdev/herdr-core/blob/main/schema/README.md
