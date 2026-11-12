# Etapa 4B — Agentes remotos por Herdr Core (ARM64)

## Objetivo y límites

Controlar desde Gentle Shell GPUI en Windows un agente **ejecutado en JaviPi**
(Linux aarch64/DietPi), usando el mismo protocolo RPC JSONL enmarcado en
base64 que se validó en 4A. El daemon Herdr de Windows usa
`terminal.run {on: "javipi", cwd: "/srv/workspaces/JaviPi", argv: [...], ...}`
y devuelve una dirección remota del tipo `javipi/tN`.

- `GENTLE_SHELL_TERMINAL_BACKEND=herdr`: agentes gestionados por Herdr.
- Sin máquina seleccionada: se mantiene el comportamiento 4A en el PC local.
- Con `GENTLE_SHELL_HERDR_AGENT_MACHINE`: los nuevos agentes se crean en
  esa máquina; nunca se envían las rutas de Windows a un agente remoto.
- `GENTLE_SHELL_TERMINAL_BACKEND=local`: backend local nativo, Machines
  desactivado, sin uso de Herdr (independientemente de las otras variables).
- Si falla el arranque remoto, **no hay fallback silencioso** a Windows.

El selector de máquina **por pestaña** es la etapa 4C. La reconexión
persistente de agentes es 4D. En 4B, reiniciar una conversación remota desde
un archivo de sesión **local de Windows** está bloqueado: creá una pestaña
nueva para pruebas. No se transfieren sesiones ni credenciales entre equipos.

## Arquitectura implementada

`pi-rpc/src/herdr_bridge.rs` contiene el puente sin dependencias de GPUI.
Lo utilizan:

- Windows: `gentle-shell-gpui.exe --herdr-agent-bridge <token>`, a través
  del wrapper que añade el contexto propio de GPUI (como en 4A).
- Linux ARM64: `gentle-shell-herdr-bridge <token>`, binario headless que
  se compila con `-p pi-rpc --bin gentle-shell-herdr-bridge`.

El puente inicia `gentle-shell --mode rpc` dentro de JaviPi, con stdin/stdout
reales del proceso hijo. La terminal Herdr transporta solo tramas marcadas,
sin pasar JSONL crudo. Se conserva la doble comprobación de `get_state`
(inicio de Pi y recorrido completo GPUI → Herdr → Pi → GPUI).

Las variables `PATH`, `HOME`, `USERPROFILE`, `GENTLE_SHELL_BIN`, etc.
de Windows **no se propagan a JaviPi**. Herdr utiliza el entorno propio del
core remoto. Solo se permiten dos overrides explícitos del administrador:

- `GENTLE_SHELL_HERDR_REMOTE_BIN` → `GENTLE_SHELL_BIN` en JaviPi.
- `GENTLE_SHELL_HERDR_REMOTE_PATH` → `PATH` en JaviPi.

Las claves de autenticación de modelos deben estar configuradas directamente
en la cuenta de JaviPi; el cliente no las copia.

## 1. Instalar el puente en JaviPi

Usar SSH o una terminal Herdr **dentro de JaviPi**, no PowerShell:

```bash
uname -m
# Esperado: aarch64. Si el sistema usa armv7l, avisar antes de compilar.

cd /srv/workspaces
git clone --single-branch --branch feature/herdr-stage4b-remote-agents \
  https://github.com/javijec/gentle-shell-gpui-herdr-core.git gentle-shell-gpui-4b
cd /srv/workspaces/gentle-shell-gpui-4b

# Cargo/Rust estable con soporte edition 2024. Evitar saturar RAM de Pi4.
CARGO_BUILD_JOBS=1 cargo build --release --locked \
  -p pi-rpc --bin gentle-shell-herdr-bridge

install -Dm755 target/release/gentle-shell-herdr-bridge \
  "$HOME/.local/bin/gentle-shell-herdr-bridge"
ls -l "$HOME/.local/bin/gentle-shell-herdr-bridge"
command -v gentle-shell
gentle-shell --version
```

Si el repositorio ya existe en esa ruta, actualizar la rama en lugar de
clonarlo de nuevo. El directorio de compilación queda en el HDD `/srv`,
evitando escribir el pesado `target/` en la microSD.

Comprobar la ruta efectiva del puente, por ejemplo:
`/home/dietpi/.local/bin/gentle-shell-herdr-bridge`. El binario debe
ejecutarse con el mismo usuario que tenga el inicio de sesión de Gentle Shell.

Si `gentle-shell` no está en el PATH del servicio Herdr, se puede pasar
la ruta real encontrada en JaviPi mediante el override específico de
`GENTLE_SHELL_HERDR_REMOTE_BIN` descrito abajo. No usar rutas Windows.

## 2. Seleccionar JaviPi desde Windows

En PowerShell, **antes de abrir GPUI**:

```powershell
cd D:\Programacion\gentle-shell-gpui-herdr-core

git fetch https://github.com/javijec/gentle-shell-gpui-herdr-core.git feature/herdr-stage4b-remote-agents
git switch -c feature/herdr-stage4b-remote-agents FETCH_HEAD

hc machine ls

$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
$env:GENTLE_SHELL_HERDR_AGENT_MACHINE = "javipi"
$env:GENTLE_SHELL_HERDR_REMOTE_CWD = "/srv/workspaces/JaviPi"
$env:GENTLE_SHELL_HERDR_REMOTE_BRIDGE = "/home/dietpi/.local/bin/gentle-shell-herdr-bridge"

# SOLO si el core remoto no encuentra gentle-shell:
# $env:GENTLE_SHELL_HERDR_REMOTE_BIN = "/srv/tools/npm-global/bin/gentle-shell"
# $env:GENTLE_SHELL_HERDR_REMOTE_PATH = "/srv/tools/npm-global/bin:/usr/local/bin:/usr/bin:/bin"

cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p gentle-shell-gpui
```

Usar el nombre exacto de `hc machine ls` (podría diferir de `javipi`).
El bridge y el CWD deben existir *en JaviPi*, no en la PC.

En la aplicación, abrir **una pestaña nueva de agente**. Pedir al agente
`pwd; uname -m; hostname` con su herramienta de shell. Se espera una ruta
bajo `/srv/workspaces/JaviPi` y arquitectura `aarch64`; un terminal
`javipi/tN` debe aparecer en `hc ls`.

Para volver a la ruta 4A (agente Herdr local), cerrar GPUI y quitar:

```powershell
Remove-Item Env:GENTLE_SHELL_HERDR_AGENT_MACHINE -ErrorAction SilentlyContinue
Remove-Item Env:GENTLE_SHELL_HERDR_REMOTE_CWD -ErrorAction SilentlyContinue
Remove-Item Env:GENTLE_SHELL_HERDR_REMOTE_BRIDGE -ErrorAction SilentlyContinue
Remove-Item Env:GENTLE_SHELL_HERDR_REMOTE_BIN -ErrorAction SilentlyContinue
Remove-Item Env:GENTLE_SHELL_HERDR_REMOTE_PATH -ErrorAction SilentlyContinue
```

Para ejecutar el respaldo completamente nativo, usar
`GENTLE_SHELL_TERMINAL_BACKEND=local`: Herdr y Machines permanecen
inhabilitados.

## 3. Matriz de validación (4B)

| Nº | Verificación | Esperado |
|----|--------------|----------|
| 49 | Compilar bridge con `cargo --release` en JaviPi ARM64 | Ejecutable headless Linux aarch64 |
| 50 | `hc machine ls` y lanzar agente remoto | Máquina conectada y dirección `javipi/tN` |
| 51 | Cargar modelos, enviar prompt, recibir streaming | Modelos y conversación de JaviPi, sin errores RPC |
| 52 | Ejecutar `pwd; uname -m; hostname` en el agente | `/srv/workspaces/JaviPi`, `aarch64`, JaviPi |
| 53 | Dos agentes remotos simultáneos | Mensajes y herramientas aislados por pestaña |
| 54 | Cerrar un agente remoto | Solo su terminal remota termina; Core y otras siguen |
| 55 | Máquina desconectada/puente inexistente | Error explícito, jamás inicia Pi local |
| 56 | Quitar configuración remota con backend herdr | Un agente nuevo vuelve a ejecutar 4A local |
| 57 | Backend local aun con variables remotas definidas | Solo procesos nativos, sin Herdr |
| 58 | Registrar `hc ls`, revisar entorno remoto | Ninguna ruta, perfil ni autenticación Windows copiada |

## Límites conocidos

- Esta rama no se ha compilado ni ejecutado todavía en Windows o JaviPi.
  Los tests incluidos son sintéticos; falta la verificación de 49–58.
- Los títulos, el navegador de archivos, el Git local y algunos indicadores
  de proyecto de GPUI todavía están ligados a Windows; en 4B no deben
  considerarse equivalentes al workspace remoto. 4C integrará la elección
  visible de máquina por pestaña y contexto remoto.
- Las sesiones persistentes de agentes y la reconexión a terminales remotas
  tras reiniciar GPUI serán tratados en 4D.
- No realizar pruebas que eliminen archivos importantes del servidor. Empezar
  con comandos de solo lectura y un directorio de trabajo conocido.
