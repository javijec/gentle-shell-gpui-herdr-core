# Herdr Stage 5D — Git remoto con escritura controlada

**Estado:** implementación propuesta en rama separada, dependiente de Stage 5C;
pendiente de CI y validación manual Windows/JaviPi. No fusionar antes de 5C.

## Alcance

Los agentes remotos nuevos pueden operar sobre el repositorio Git Linux
seleccionado, dentro de su propia sesión Herdr:

- **Changes:** Stage de un solo archivo y Unstage de un solo archivo.
  Se preservan los contenidos del worktree al deshacer el staging.
- **Commit:** mensaje explícito y confirmación en dos pasos; incluye
  únicamente los cambios que *ya* estén staged, no los cambios sin stage.
- **Branches:** crear una rama local en el HEAD actual sin cambiar de rama.
  Cambiar a una rama local con confirmación y únicamente si el índice y
  el árbol de trabajo están completamente limpios, incluidos no rastreados.
- **Refresh:** tras una escritura exitosa, nuevo snapshot del estado remoto.

No se ejecutan comandos Git en Windows ni nuevos procesos de agente Pi.
El canal reutiliza los frames privados del puente Node existente en Herdr.
Un agente remoto ya abierto con el puente anterior a 5D sigue siendo de
solo lectura: hay que **abrir un agente nuevo** para usar estas acciones.

## Protocolo

El snapshot `herdr_git_snapshot` de 5C conserva `readOnly:true` para
compatibilidad del lector y añade `writesEnabled:true`, `headOid`,
`currentBranch` y `local` en cada rama. No implica que ninguna lectura
modifique Git. Las escrituras tienen comandos privados separados:

| Tipo | Campos adicionales |
|---|---|
| `herdr_git_stage` | `path`, `expectedIndex`, `expectedWorktree` |
| `herdr_git_unstage` | `path`, `expectedIndex`, `expectedWorktree` |
| `herdr_git_commit` | `summary`, `description` opcional |
| `herdr_git_create_branch` | `branch` |
| `herdr_git_switch_branch` | `branch` |

Todas las escrituras deben incluir `root`, `toplevel`,
`expectedHead`, `expectedBranch`, `expectedIndexToken`, además del `id` normal de RPC.
La ruta del repositorio debe coincidir con la obtenida por el puente.
El puente verifica de nuevo `HEAD`, la rama actual y la huella SHA-256 del diff crudo staged (`indexToken`) antes de mutar. Así, otro proceso no puede cambiar silenciosamente el índice staged entre el snapshot y la petición. La huella se calcula con lectura acotada; si supera el límite de 200 cambios staged, se deshabilitan las escrituras hasta reducir el conjunto.
Las acciones por archivo también comprueban el estado Git observado
(`index` / `worktree`), y rechazan rutas escapadas, externas, enlaces
simbólicos y directorios.

Sólo se permiten los verbos fijos enumerados, nunca comandos Git
arbitrarios. `git` se ejecuta con `shell:false`, sin editor interactivo,
paginador, firma de commit ni hooks. Las escrituras de cada repositorio
se serializan en un único puente. Git maneja sus bloqueos de índice
obligatorios y los conflictos con otros procesos.

Los commits no utilizan `--all`, `--amend`, `-a`, `reset` ni `push`.
La GUI deshabilita Commit cuando el listado visible está truncado, para
evitar aprobar un conjunto de cambios incompleto. El botón de commit y
las operaciones de rama requieren un segundo clic explícito.

## Fuera de alcance

Push, pull, fetch, force push, merge, rebase, cherry-pick, stash, borrado
de ramas, descarte de cambios, reset, stage-all, unstage-all, checkout de
ramas remotas y creación de worktrees remotos.

## Pruebas

```powershell
git switch feature/herdr-stage5d-remote-git-write
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

En Windows se omiten los tests de Git de Linux. **Repetir el comando
`node --test` en JaviPi** con los archivos de 5D (en un checkout de
prueba, sin modificar el repositorio activo). Las pruebas usan
repositorios temporales y limpian sus carpetas.

## Validación visual en JaviPi

1. Crear agente **nuevo** apuntando a un proyecto Git Linux.
2. Abrir Changes, modificar un archivo y Stage. Verificar en Git Linux
   que el index contiene el cambio y el contenido del archivo no cambió.
3. Unstage y verificar que el worktree conserva el contenido.
4. Stage, modificar el archivo de nuevo, Commit con dos clics.
   El commit debe incorporar sólo la versión staged, no la edición posterior.
5. Con un árbol limpio, crear rama local. Probar cambio de rama y volver.
6. Con cualquier archivo sin seguimiento, el cambio de rama debe fallar
   con aviso explícito y sin mover HEAD.
7. Cambiar el HEAD desde otro terminal tras el snapshot: la escritura
   debe fallar por conflicto hasta presionar Refresh.
8. Probar archivos vía enlaces simbólicos y fuera del root, archivo
   sin cambios, nombre inválido y agente antiguo 5C. Ninguno debe escribir.
9. Verificar que las operaciones no aparecen como mensajes/comandos del
   agente Pi, y que cerrar el panel no finaliza la sesión.

**Nota:** la verificación de estado por archivo protege de un estado Git
desactualizado, pero no detecta necesariamente cambios externos del
contenido cuando los indicadores de estado siguen siendo iguales. Git
está compartido con otros procesos; antes de confirmar revisá los
archivos staged y mantené los repositorios de trabajo bajo control.
