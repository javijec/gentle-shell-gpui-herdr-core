# Etapa 4B — Agentes remotos sin instalar ni compilar nada en JaviPi

## Qué cambia

Windows conserva Gentle Shell GPUI y su interfaz de agentes. Herdr Core
ejecuta los agentes en Linux (JaviPi) y transporta el protocolo RPC.
**No existe una instalación separada de puente en JaviPi.**

La app incluye `crates/pi-rpc/src/herdr_remote_bridge.cjs`. Cuando se abre
una pestaña remota, GPUI llama a `terminal.run` con la máquina seleccionada,
un directorio remoto y el comando `node --eval <código integrado> <token>`.
Herdr transmite ese comando **directamente** al equipo remoto; el código
se ejecuta en memoria, sin instalar paquetes npm, sin Git, sin Rust ni archivos
temporales del puente en la Raspberry Pi.

El puente ejecuta el `gentle-shell --mode rpc` **ya existente en JaviPi**
y usa sus tuberías stdin/stdout. Node.js es una dependencia que ya requiere
Gentle Shell. El agente emplea **la configuración y autenticación propias**
de JaviPi; GPUI no copia credenciales del PC.

Los paquetes de entrada/salida usan `GS4A1|<token>|<tipo>|<base64>` sobre
la PTY Herdr, bloques de hasta 96 bytes y las dos comprobaciones de
`get_state` heredadas de 4A: se verifica que Pi funcione y que el camino
GPUI → Herdr → JaviPi → GPUI sea bidireccional. Los errores no activan
otro agente local por detrás.

## Qué tiene que existir en JaviPi

**Solo lo que ya usás:**

- Herdr Core instalado y registrado en el Herdr Core de Windows.
- Node.js funcionando en la sesión que Herdr inicia en JaviPi.
- Gentle Shell instalado y funcional en esa misma cuenta de JaviPi.
- Tus modelos/autenticaciones configurados en JaviPi.

**No hace falta compilar el proyecto ni instalar un puente ARM64.**

Si Herdr no encuentra `node` o `gentle-shell` porque su PATH es diferente
del PATH de la terminal interactiva, podés especificar la ubicación real
con los overrides de la sección de diagnóstico desde Windows, sin
instalaciones adicionales.

## Prueba desde Windows

Asegurate primero de que el PR #7 esté actualizado (en la rama de pruebas,
no en `main`):

```powershell
git fetch https://github.com/javijec/gentle-shell-gpui-herdr-core.git feature/herdr-stage4b-remote-agents
git switch feature/herdr-stage4b-remote-agents
git merge --ff-only FETCH_HEAD

cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs

hc machine ls
```

Si Herdr registra la Raspberry con el nombre exacto `javipi`:

```powershell
$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
$env:GENTLE_SHELL_HERDR_AGENT_MACHINE = "javipi"

# Por defecto, JaviPi usa /srv/workspaces/JaviPi.
# Solo hace falta cambiarlo si querés abrir otro directorio:
# $env:GENTLE_SHELL_HERDR_REMOTE_CWD = "/srv/workspaces/PoeTradePlus2"

# Ya NO se utiliza un ejecutable remoto:
Remove-Item Env:GENTLE_SHELL_HERDR_REMOTE_BRIDGE -ErrorAction SilentlyContinue

cargo run --locked -p gentle-shell-gpui
```

Abrí una pestaña nueva de agente y comprobá: modelos, respuesta a un
mensaje y ejecución de herramientas. Para confirmar dónde se ejecuta,
pedí `pwd; uname -m; hostname` con una herramienta del agente; en JaviPi
debés ver `/srv/workspaces/JaviPi`, `aarch64` y su hostname. En Windows,
`hc ls` debe mostrar una terminal remota `javipi/tN` etiquetada
`gentle-shell=rpc-agent`. El agente Pi debe aparecer **en JaviPi**, no
como un nuevo proceso local en Windows.

La selección por máquina es global y solo afecta pestañas nuevas
en esta etapa 4B. Un selector explícito por pestaña queda para 4C.

## Recuperar los mismos agentes después de cerrar GPUI

El archivo existente de espacios de trabajo de GPUI
(`workspaces.json`) ahora guarda también la dirección Herdr de cada
**agente**, además de la dirección de las terminales normales. No guarda
claves de proveedores ni transfiere archivos de sesión entre máquinas.

1. Creá uno o dos agentes remotos, esperá que carguen modelos y registrá
   sus direcciones `javipi/tN` mediante `hc ls`.
2. Cerrá **GPUI normalmente**. Herdr y los procesos Pi de JaviPi siguen vivos;
   solo se desconectan los visores. No hace falta terminar sus turnos.
3. Volvé a abrir GPUI. Las pestañas guardadas reusan exactamente las mismas
   direcciones Herdr y el mismo token RPC, sin hacer otro `terminal.run`.
   El programa valida un `get_state` completo y solicita `get_messages`
   para recuperar el historial reciente.
4. Confirmá con `hc ls` que las direcciones no cambiaron y que no apareció
   una segunda copia del agente. Enviá otro mensaje al mismo agente.
5. Cerrá **una pestaña** y comprobá que se termine solo esa terminal. Cerrar
   una pestaña significa terminar el agente; cerrar GPUI significa desconectar
   su vista.

Para agentes remotos que quedaron vivos con las compilaciones **anteriores**
de 4B (que no guardaban la dirección), mantené
`GENTLE_SHELL_HERDR_AGENT_MACHINE=javipi` al abrir GPUI. Se consultan las
terminales remotas vivas etiquetadas `gentle-shell=rpc-agent`, se verifica
su protocolo y se abren pestañas recuperadas **sin iniciar otro proceso**.
Solo se incorporan las que no tengan ya una pestaña con dirección guardada.
Los paneles vacíos antiguos pueden seguir presentes hasta que los cierres.

Si JaviPi está desconectada o la terminal guardada finalizó, se muestra un
error en esa pestaña; **no se ejecuta otro agente en Windows ni se reemplaza
silenciosamente el proceso**. La recuperación automática tras una pérdida
de conexión prolongada durante la ejecución sigue siendo objetivo de 4D.



## Diagnóstico si Herdr no encuentra Node o Gentle Shell

Desde una terminal Herdr conectada a JaviPi (o su terminal SSH existente),
podés consultar:

```sh
command -v node
node --version
command -v gentle-shell
gentle-shell --version
```

Si faltan únicamente del PATH del servicio remoto, establecé desde
PowerShell las rutas **Linux** verificadas en JaviPi (solo cuando haga falta):

```powershell
$env:GENTLE_SHELL_HERDR_REMOTE_NODE = "/usr/bin/node"
$env:GENTLE_SHELL_HERDR_REMOTE_BIN = "/srv/tools/npm-global/bin/gentle-shell"
$env:GENTLE_SHELL_HERDR_REMOTE_PATH = "/srv/tools/npm-global/bin:/usr/local/bin:/usr/bin:/bin"
```

No supongas que esas rutas existen: consultalas en JaviPi. No uses rutas
`C:\\` ni variables `HOME` de Windows. Si la máquina no está conectada,
no hay un fallback silencioso al agente local.

## Si el arranque de Pi es lento en JaviPi

El puente Node ahora concede hasta 60 segundos a Gentle Shell para
contestar el primer `get_state`; GPUI concede hasta 90 segundos a ese
arranque remoto, mientras que el agente local 4A conserva sus 30 segundos.
No se pasa automáticamente a un agente Windows.

Si expira, el error muestra estadísticas limitadas del stdout inicial,
sin copiar contenido de mensajes: `stdout_records`, `invalid_json`
y `last_type`, además de las últimas líneas de stderr. Por ejemplo:

- `stdout_records=0`: el proceso todavía no generó una línea stdout;
  podría estar bloqueado en carga de extensiones o inicialización.
- `invalid_json>0`: se está escribiendo texto que no cumple RPC JSONL
  en stdout (p. ej. banner de CLI).
- `last_type=...`: se recibieron eventos, pero no la respuesta
  `get_state` con el ID correlacionado.

Para revisar Node, Gentle Shell y la identidad del proceso remotamente,
**solo desde Windows** mediante Herdr, sin acceder por SSH ni compilar:

```powershell
hc exec --on javipi --timeout 30 node --version
hc exec --on javipi --timeout 30 gentle-shell --version
hc exec --on javipi --timeout 30 id -un
hc exec --on javipi --timeout 30 pwd
```

Si `gentle-shell --version` funciona pero el arranque RPC vuelve a
bloquearse, el problema queda localizado en la inicialización de Pi/Gentle
Shell en esa cuenta; se puede investigar las extensiones sin modificar
el transporte ni instalar software.

## Desactivar el modo remoto

Para usar otra vez 4A en el mismo PC (Herdr local):

```powershell
Remove-Item Env:GENTLE_SHELL_HERDR_AGENT_MACHINE -ErrorAction SilentlyContinue
Remove-Item Env:GENTLE_SHELL_HERDR_REMOTE_CWD -ErrorAction SilentlyContinue
Remove-Item Env:GENTLE_SHELL_HERDR_REMOTE_NODE -ErrorAction SilentlyContinue
Remove-Item Env:GENTLE_SHELL_HERDR_REMOTE_BIN -ErrorAction SilentlyContinue
Remove-Item Env:GENTLE_SHELL_HERDR_REMOTE_PATH -ErrorAction SilentlyContinue

$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
```

Para respaldo totalmente nativo:

```powershell
$env:GENTLE_SHELL_TERMINAL_BACKEND = "local"
```

El backend `local` ignora toda configuración remota y no utiliza Herdr.

## Matriz de aceptación de 4B

| Nº | Prueba | Resultado esperado |
|----|--------|--------------------|
| 49 | Iniciar remoto sin Rust, Git ni bridge instalado en JaviPi | El puente Node integrado inicia en memoria |
| 50 | `hc machine ls`, GPUI y `hc ls` | Dirección de terminal `javipi/tN` |
| 51 | Modelos, prompt y streaming | Modelos y mensajes remotos correctos |
| 52 | `pwd; uname -m; hostname` | Ruta Linux JaviPi y CPU aarch64 |
| 53 | Dos agentes remotos | Canales y respuestas independientes |
| 54 | Cerrar un agente | Solo termina su terminal Herdr; Core y otros siguen |
| 55 | Máquina desconectada o Node inexistente | Error explícito sin abrir agente local |
| 56 | Quitar selección remota en Herdr | Nuevas pestañas vuelven al agente 4A |
| 57 | Elegir backend `local` con variables remotas | Sin procesos ni conexiones Herdr |
| 58 | Inspeccionar entorno remoto | Ninguna credencial o ruta Windows transferida |

## Pruebas de persistencia (4B-P)

| Nº | Acción | Esperado |
|----|--------|----------|
| P1 | Abrir 2 agentes remotos y anotar `hc ls` | Dos direcciones distintas `javipi/tN` |
| P2 | Salir normalmente de GPUI | Ambos agentes siguen en Herdr |
| P3 | Volver a abrir GPUI | Dos pestañas enlazadas a las mismas direcciones, sin nuevas terminales |
| P4 | Consultar el historial y enviar otro prompt | Mismos modelos y mensajes, respuesta RPC válida |
| P5 | Cerrar una pestaña recuperada | Termina solo su agente; el otro sigue vivo |
| P6 | Cerrar una pestaña recuperada antes de activarla | También termina el proceso elegido |
| P7 | Reabrir con JaviPi desconectada | Error de conexión, ningún fallback local |
| P8 | Recuperar agentes huérfanos de una compilación 4B vieja | Pestañas nuevas, sin duplicar procesos |
| P9 | Cambiar a backend nativo `local` | Herdr no se contacta y las identidades remotas quedan guardadas |

La recuperación tras reiniciar GPUI es deliberadamente distinta de 4D:
el cliente que vuelve a abrir consulta Herdr y reatacha el terminal existente;
no se ha implementado todavía el rebinding durable ante cada interrupción
arbitraria de un flujo RPC en plena ejecución.

## Límites

- La rama contiene la implementación y pruebas sintéticas; todavía hay que
  correr Clippy/tests de workspace y una prueba real Herdr + JaviPi.
- Las rutas del sistema de archivos y estado de Git mostrados en algunas
  partes de la UI pueden seguir refiriéndose al PC Windows en 4B. En 4C se
  integrarán el contexto y la elección de máquina por pestaña.
- No se reanudan archivos de sesión Windows dentro de JaviPi: abrir
  una pestaña nueva. La persistencia/reconexión de agentes es 4D.
- El código del puente viaja como argumento a `node --eval`; no contiene
  tokens de proveedores ni claves. Herdr transmite ese comando únicamente
  a la máquina elegida.
