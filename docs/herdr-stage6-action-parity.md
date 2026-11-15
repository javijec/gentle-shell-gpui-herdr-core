# Stage 6 — Enrutamiento por agente y paridad funcional local/Herdr

**Rama:** `feature/herdr-stage6-action-parity` — PR #27 (borrador).
**Base:** GPUI 0.1.19 + Stage 5 en `main`.
**Estado:** Implementación en rama; faltan compilación, tests y aceptación manual. **No fusionar** sin ese resultado.

## Invariantes de producto

1. Cada acción usa el agente que la originó: la máquina, el terminal Herdr y su directorio Linux. Ningún comando remoto puede ejecutar Git o acceder a archivos en Windows.
2. Los controladores nativos (Files/Changes/Branches/History, paleta, consola) reutilizan presentación y modelos; los backends son distintos donde cambian SO, transporte o permisos.
3. Nunca hay *fallback* silencioso local cuando JaviPi está desconectada, el puente es viejo o falta el directorio de trabajo.
4. Una acción tardía tras cambiar de agente se rechaza (máquina + raíz + terminal); no se reinterpreta bajo la nueva selección.
5. Los nuevos RPC privados se interceptan en el puente Node dentro de Herdr; no pasan como `bash` al agente, no generan tarjetas Shell ni procesos Pi adicionales.

## Matriz de acciones

| Acción | Agente Windows | Agente JaviPi |
| --- | --- | --- |
| Paleta → Files | `walk_files` local | `herdr_palette_snapshot`: `git ls-files` acotado o escaneo POSIX acotado, 2000 archivos máx. |
| Paleta → Branches | `git branch` local | ramas de `herdr_palette_snapshot`, vinculadas a terminal/raíz |
| Abrir archivo en paleta | `open_file(PathBuf)` | `open_remote_file(String)` validado, sin PathBuf de Windows |
| Cambiar rama desde paleta | confirmación local | abrir Branches remoto y usar el segundo clic + guardas de Stage 5 |
| Paleta → Commit History | `HistoryView` con `git::history` | el mismo `HistoryView` y gráfico con `herdr_git_history` |
| `/gentle:changes` | Files dock derecho, Changes | ChangesView remoto en dock derecho, no Machines ni Git Windows |
| Nueva pestaña de agente | backend local | nueva sesión en la máquina y cwd Linux del agente activo |
| Nueva terminal | PTY local | `terminal.run` remoto en máquina y cwd seleccionados |
| Dividir en agente/terminal | nuevo pane local | agente remoto en split; terminal Herdr nueva en pestaña |
| Configuración de Profiles | Desktop RPC o archivos locales heredados | **sólo** Desktop RPC de la sesión remota; se prohíbe leer config Windows |
| Settings / Providers | configuración de la instalación Windows | se identifica explícitamente como local; no reinicia ni reconfigura agentes JaviPi |
| Architecture Docs | GUI local de instalación/chequeo | abre directorio `docs/` Linux mediante Files; generación por agente remoto |
| Sessions (archivos de sesiones guardadas) | navegador JSONL Windows | accesos a terminales remotas activas mediante Machines; no lectura falsa de sesiones Windows |
| Engram / Memory | navegador del Engram local | aviso explícito: requiere consultas desde el agente JaviPi, sin mostrar memorias Windows como remotas |

Las tres últimas diferencias (gestión completa de Docs, sesiones persistidas que ya no están vivas y GUI Engram remota) requieren APIs del proveedor del servicio; **no se simula paridad** usando recursos Windows.

## RPC privados y límites

- `herdr_git_history`: límite 1–800 commits, refs validados, salida Git acotada a 384 KiB, no hooks/mutaciones, usa mismo formato `gentle_core::git::log::LOG_ARGS` para alimentar el gráfico nativo.
- `herdr_palette_snapshot`: 2000 archivos, 120 refs; lectura limitada al agente/proyecto remoto, sin symlinks, rutas externas o comandos Git arbitrarios.
- Sesiones remotas antiguas con puente 5F no conocen estos verbos. Abrir **un agente nuevo** desde esta versión; los anteriores siguen funcionando para las capacidades anteriores.
- Las respuestas sin conexión y las rutas inválidas se rechazan; nunca se dispara trabajo sobre la máquina local.

## Validación obligatoria

Windows (checkout de la rama):

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
cargo run -p gentle-shell-gpui --locked
```

Linux/JaviPi (tests aislados en repos temporales; nunca el proyecto real):

```sh
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

### Aceptación manual

1. Local Windows: History, Changes, búsqueda Files/Branches, editar archivo, crear agente y terminal, switch con confirmación.
2. JaviPi: misma secuencia, `Ctrl+K → History` muestra sólo commits de JaviPi y el mismo gráfico; cambio de rama desde la paleta requiere confirmación existente de Branches.
3. `/gentle:changes` abre Changes del remoto, sin mover/cerrar Machines y sin tarjetas de herramientas Git creadas automáticamente en el chat.
4. Con dos agentes sobre un mismo repo: iniciar una búsqueda, cambiar de agente, elegir un resultado anterior: rechazar con aviso explícito.
5. Con JaviPi desconectada: paleta no lista Windows, History indica fallo del puente y no se ejecutan acciones locales.
6. `Ctrl+T` y New Agent dentro de una sesión JaviPi crean únicamente terminal/agente remoto; comprobar `hc ls` y que no crecen los procesos locales inesperadamente.
7. Profiles remotos: cambios por RPC del propio agente; si es legado, error explícito en vez de `profiles.json` de Windows.
8. Cambios de Settings/Providers Windows no reinician procesos ni invalidan el auth del agente JaviPi.
9. Crear archivo remoto, Stage/Unstage, cambio de rama, desconexión/reconexión, cambiar de máquina: confirmar no-regresión Stage 5.
10. Cerrar GPUI y volver a abrir: los agentes Herdr recuperados conservan identidad; History remoto se vuelve a abrir desde el agente, no se restaura por ruta Windows.

## Cierre

La parte de código está propuesta; la aprobación requiere pruebas Windows + Linux/JaviPi y una pasada de UI. Los tests sintéticos no sustituyen esa validación. `main` continúa estable hasta completar la aceptación.
