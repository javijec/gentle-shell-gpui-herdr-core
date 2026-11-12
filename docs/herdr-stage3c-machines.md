# Etapa 3C — Machines (primer entregable)

Panel Machines nativo de GPUI, integrado sobre la etapa 3B. Descubre máquinas
y terminales con el protocolo público de Herdr Core, sin ejecutar SSH ni
comandos de shell en el hilo gráfico.

## Interfaz

Solo cuando `GENTLE_SHELL_TERMINAL_BACKEND=herdr`, abrir con el botón de
terminal de la barra superior o desde la paleta de comandos
(Ctrl+K > Machines, o Cmd+K en macOS). En modo `local` (o sin seleccionar
explícitamente Herdr), Machines no aparece ni se puede invocar.

- Grupo local con el nombre informado por `hello.core.name`.
- Grupos remotos (nombre, host, target y estado real de `machine.list`).
- Estados como `connected`, `connecting`, `needs_login` y `unreachable`;
  no se muestra `online` si Herdr no lo informó.
- Listado por máquina de `terminal.list` con dirección `tN` o
  `machine/tN`, nombre, cwd y estado.
- Open: adjuntarse a una terminal existente en nueva pestaña; no ejecuta
  `terminal.run` y no crea otra sesión.
- New shell: invoca `terminal.run {on: machine, no_parent: true, wait: false}`
  en Herdr Core y abre una pestaña con la dirección devuelta, tanto en el
  core local como en uno remoto.
- Connect: solicita `machine.connect` para una máquina desconectada.
- Refresh: consulta `machine.list` y `terminal.list` otra vez.
- Errores de Herdr visibles sin bloquear el resto de GPUI.

Los backends son mutuamente excluyentes durante cada ejecución de GPUI:

- `herdr`: todas las terminales nuevas, las restauradas y las que se abren
  desde Machines usan Herdr Core; la vista Machines está habilitada.
- `local`: todas las terminales abiertas usan un PTY nativo. Machines está
  oculto de la barra y paleta; incluso si se intenta invocar internamente,
  no realiza descubrimientos, conexiones ni comandos `hc`.
- Variable ausente o valor desconocido: comportamiento local, sin Machines.

Las direcciones Herdr guardadas permanecen en el archivo de workspace al
abrirlo con backend `local`. El proceso PTY mostrado es local, pero volver
a ejecutar GPUI con backend `herdr` permite recuperar las direcciones
originales sin perderlas por un guardado intermedio.

## Validación en Windows

    git fetch https://github.com/javijec/gentle-shell-gpui-herdr-core.git feature/herdr-stage3c-machines
    git switch -c feature/herdr-stage3c-machines FETCH_HEAD
    cargo test --locked -p herdr-client
    cargo test --locked -p gentle-shell-gpui
    cargo clippy --workspace --all-targets --locked -- -D warnings
    $env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
    cargo run --locked -p gentle-shell-gpui

Para verificar la exclusión estricta en Windows:

    $env:GENTLE_SHELL_TERMINAL_BACKEND = "local"
    cargo run --locked -p gentle-shell-gpui

Abrir terminales con "+" y comprobar que se crean como PTY nativo de
Windows. No debe aparecer el botón Machines ni el comando Machines en
Ctrl+K. No debe arrancar ningún proceso `hc connect` por GPUI ni crearse
ninguna sesión Herdr. Si se cambia a `herdr` y se reinicia GPUI, Machines
debe aparecer y tanto "+" como New shell deben crear sesiones Herdr.

Si ya tenés esta rama local:

    git fetch https://github.com/javijec/gentle-shell-gpui-herdr-core.git feature/herdr-stage3c-machines
    git merge --ff-only FETCH_HEAD

### Lista de comprobación

30. Machines se abre desde la barra y desde Ctrl+K.
31. Herdr local aparece con su nombre; Refresh no congela GPUI.
32. Las terminales locales actuales coinciden con `hc ls`.
33. Open de una terminal local usa exactamente su dirección (sin duplicarla).
34. New shell local crea solo una nueva dirección y funciona teclado/TUI.
35. Al cerrar y reabrir GPUI, la dirección de esa shell se restaura.
36. Si hay una máquina registrada, muestra su estado `connected` u offline
    como lo informa `hc machine ls`.
37. Una máquina conectada presenta `machine/tN`, y Open adjunta la sesión.
38. New shell remota crea una dirección `machine/tN` y abre su PTY.
39. Si está desconectada, Connect pide a Herdr la conexión, y Refresh refleja
    lo que reporte su estado.
40. Sin `hc` accesible, Machines muestra un error en lugar de bloquear GPUI.
41. Con backend `local`, todas las terminales usan PTY nativo; Machines
    no aparece en la barra ni en Ctrl+K, no hay conexiones ni invocaciones
    de `hc`. Las direcciones Herdr previamente guardadas persisten para
    una ejecución posterior con backend `herdr`.

Las comprobaciones 36–39 requieren una máquina remota configurada mediante
Herdr; si no existe, deben marcarse NP, no como aprobadas.

## Límites intencionales

- La lista se actualiza al pulsar Refresh (no son eventos en tiempo real).
- No se agregan, renombran ni eliminan máquinas desde GPUI: `hc machine`
  sigue administrando esas configuraciones.
- No se abren terminales `finished` con Open.
- `machine.connect` puede solicitar credenciales según el transporte; en
  ese caso se muestra el error/estado del core y debe completarse el login
  mediante la herramienta apropiada.
- Algunas conexiones remotas pueden bloquear el handshake: el cliente aún
  necesita timeouts explícitos.
- Falta validación de integración en sistemas distintos de Windows, y CI.

API: https://github.com/herdrdev/herdr-core/blob/master/schema/hc.openrpc.json
