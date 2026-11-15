# Stage 5 — Paridad Files/Git local y Herdr: cierre de implementación

**Fecha:** 2026-11-15
**Estado:** Implementado en PR #24 (rama `feature/herdr-stage5-completion`), sobre
la integración ya fusionada de PRs #15–#23. **Pendiente de compilación estricta,
pruebas del bridge en JaviPi y aceptación Windows.** No se fusiona ni se
declara validado este nuevo código hasta completar esa batería.

## Un solo panel, backends distintos

- **Files:** un dock con filas compartidas y exploración remota virtualizada,
  navegación, teclado y portapapeles de archivos por backend.
- **Changes:** la misma instancia de `ChangesView`, alimentada por
  `WorkTree` remoto y eventos con estado de Git verificado.
- **Branches:** la misma instancia de `RefsView`, alimentada por
  `RefsSnapshot` remoto con ramas, remotos, tags, stashes, worktrees y
  submódulos.
- **Editor/Diff:** componentes GPUI y modelo de edición compartidos; el
  controlador remoto de sesión y estado de lectura/escritura sigue siendo
  distinto del controlador de archivos local.
- **Rutas:** ningún valor POSIX de JaviPi se convierte en `PathBuf` para
  ejecutar operaciones de Windows. Ninguna operación de Files/Git vuelve
  a Pi como bash o comandos de agente.

## Operaciones agregadas en el cierre (PR #24)

| Área | Comportamiento |
|---|---|
| Files: copia recursiva | Máximo 500 entradas, 64 MiB y 24 niveles; rechaza symlinks y entradas especiales; nunca sobrescribe destinos |
| Files: Trash / Undo | Confirmación doble, token aleatorio, papelera privada junto al directorio del proyecto; restauración sin pisar archivos existentes |
| Files: ignorados | Mostrar/ocultar dotfiles y coincidencias de `.gitignore`, con `git check-ignore --stdin -z`; `.git` no se muestra |
| Git: stash | Push desde Changes; Apply/Pop/Drop desde Refs, con confirmación, OID del stash y protecciones de HEAD/índice |
| Git: sincronización | Fetch, Pull fast-forward y Push de una sola rama, todos con confirmación y transporte HTTPS/SSH validado; nunca `force` |
| Git: worktrees | Crear nueva rama y worktree, enlazar rama existente, eliminar worktree limpio o podar entradas obsoletas, dentro de carpeta privada |
| Git: Discard | Estado exacto del archivo, doble confirmación; untracked pasa a Trash con Undo, los archivos trackeados se restauran según la cara elegida |

Estos límites son restricciones explícitas de seguridad, no errores a
compensar con comandos Windows o shell remotos.

## Protecciones que se deben verificar

1. **Límites de máquina y directorio:** rechazar rutas externas, `..`,
   symlinks de escape, `.git`, nombres privados y raíces `/`.
2. **No sobrescritura:** crear/copiar/renombrar sin reemplazar destino.
   Copias fallidas no dejan carpetas parcialmente creadas.
3. **Trabajo Git en revisión:** cada escritura compara HEAD, rama e índice;
   operaciones sobre un archivo verifican XY. Stash comprueba OID.
4. **Acciones destructivas:** confirmar Discard, Drop, Pop, Trash,
   modificación de worktrees y sincronización; nunca `--force`.
5. **Trabajo simultáneo:** cambiar de agente, cerrar/reabrir y reconectar
   no debe aplicar respuestas o revisiones del backend equivocado.
6. **Rendimiento:** directorios grandes usan una lista GPUI virtualizada,
   sin buffer ilimitado de Git.
7. **Permisos:** Trash vive en una carpeta privada del mismo volumen
   junto al workspace; si el usuario no tiene permisos para crearla, la
   operación debe **fallar sin borrar** el archivo. No se prometen
   borrados o recuperación en dispositivos distintos.

## Pruebas integrales pendientes

Windows, checkout exclusivo de la rama final:

```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p gentle-shell-gpui --locked
```

JaviPi/Linux, mismo código y repositorio Git descartable:

```bash
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

**Aceptación manual conjunta** (un solo ciclo, no uno por commit):

- Windows local: conservar FileTreeView, ChangesView, RefsView, editor,
  diff, stash, worktree y operaciones de contexto.
- JaviPi Files: navegación, atajos, copiar/duplicar carpeta, límites,
  ocultar/mostrar ignorados, mover a Trash y deshacer, confirmar colisiones
  y rechazos por symlink/ruta externa.
- JaviPi Git: Stage/Unstage individual/masivo, commit, cambio de rama,
  stash Push/Apply/Pop/Drop, Discard y Undo de untracked, worktrees.
- Red Git: comprobar rechazo de URL `file://` y `ext::`; usar un
  repositorio *descartable* con remoto HTTPS/SSH controlado para Fetch,
  Pull fast-forward y Push sin force.
- Recuperación: dos máquinas alternándose, desconexión/reconexión,
  respuestas tardías, sin operaciones ni editor apuntando al host equivocado.

## Estado de integración

PRs #15–#23 ya fusionados en `main`. PR #24 es la única capa adicional
de paridad; **no mezclar cambios de upstream v0.1.19 antes de completar
la validación y fusionar este cierre**.

**Pendientes intencionales** fuera de Stage 5: transferencia de archivos
entre agentes/volúmenes, edición de binarios o archivos mayores de los
límites explícitos y refactor para convertir los dos controladores
FileTreeView/RemoteFilesView en un solo tipo Rust. El objetivo de Stage 5
es paridad observable y backend seguro, no eliminar cada adaptador
específico de plataforma.
