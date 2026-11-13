# Herdr 5C — Git remoto (solo lectura)

**Estado:** implementación inicial en rama separada; sin fusionar y pendiente de
validación en Windows y JaviPi. Base: `main` después de 5B.

## Objetivo

Con un agente de JaviPi seleccionado, el panel derecho muestra las mismas
secciones básicas que el local: **Files / Changes / Branches**.

- **Files:** explorador remoto de 5B intacto. El editor Linux sigue en un
  panel central, con el guardado por revisión SHA-256.
- **Changes:** lista de archivos modificados, nuevos y eliminados de Git,
  con estado de index y worktree. Click sobre un archivo: abre su diff
  remoto en el **visor central Diff**, sin ejecutar Git en Windows.
- **Branches:** ramas locales y de seguimiento remoto, con HEAD identificado.
  No hay checkout ni menús de mutación.
- **Refresh:** lee un nuevo snapshot; la primera entrega usa actualización
  explícita en lugar de sondeo Git constante.

El agente de origen nunca cambia al seleccionar un archivo diff. La vista
remota hereda el `SessionView` concreto y conserva el proyecto Linux,
incluso si el teclado pasa al panel central. Cerrar diff no cierra agentes.

## Protocolo

El puente remoto de Node, ya inyectado por Herdr, agrega:

- `herdr_git_snapshot`: entrada `{type, id, root}`; salida
  `{toplevel, files:[{path,index,worktree}], filesTruncated,
    branches:[{name,current}], branchesTruncated, readOnly:true}`.
- `herdr_git_diff`: entrada `{type,id,root,path}` con path Linux absoluto;
  salida `{toplevel,path,patch,readOnly:true}`.

Ninguna operación se reenvía al RPC de Pi ni ejecuta `bash` o Git de Windows.

El puente llama al ejecutable `git` mediante `execFile` con parámetros
fijos y `shell:false`, `--no-optional-locks`, `core.fsmonitor=false`,
`--no-ext-diff`/`--no-textconv` en diffs, sin paginador ni prompts, con
timeout de 8 s y buffer máximo de 384 KiB.

Solo se permiten proyectos Git cuyo `toplevel` esté dentro del cwd del
agente. **Si el agente está en un subdirectorio de un repo padre**, hay que
iniciar un agente en la raíz del repositorio para consultarlo. Esta regla
evita ver datos fuera del área de trabajo elegida. La validación de
`root`/realpath se realiza en Linux; ningún path relativo o Windows
habilita una consulta.

Las respuestas se acotan a 200 archivos, 120 ramas y 96 KiB de diff.
Cuando hay más, el panel informa del truncamiento. Un archivo sin cambios
no puede pedirse arbitrariamente mediante `herdr_git_diff`: el puente
revalida que figure en `git status`. Los archivos sin seguimiento usan
`git diff --no-index` sobre un archivo regular, existente y menor de
128 KiB, verificando que no escape del root por symlink.

Los diffs de cambios indexados y sin indexar se muestran juntos contra
`HEAD` cuando existe. Para un repositorio sin primer commit, se usa un
fallback hacia el diff indexado (o el diff no indexado si aquél está vacío).
No se genera ni aplica un parche, ni se escribe el repositorio.

Los agentes cuyo puente es previo a 5C continúan disponibles: el panel
muestra error de comando no soportado y no reinicia ni altera la sesión.

## No incluido

- Stage/unstage, discard, commit, amend, push, pull o checkout;
- manejo o aplicación de stashes; creación de ramas;
- operaciones Git sobre proyectos locales desde el panel remoto;
- restablecimiento persistido de vistas diff tras reiniciar GPUI;
- diffs binarios grandes o archivos fuera de los límites;
- Git en un repo padre de la carpeta de lanzamiento del agente.

## Validación antes de merge

### Windows (desde la raíz de la rama 5C)

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

### JaviPi/Linux

Los tests Git sobre repositorios temporales se omiten en Windows. Ejecutar
`node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs` con los
archivos de la rama 5C en JaviPi o Linux. Requiere Git instalado en PATH.
No ejecutarlo en un repositorio importante si no se sabe qué script se está
usando: el test ya genera repositorios temporales y los elimina.

### Comprobaciones visuales

1. Abrir un agente **nuevo** en `/srv/workspaces/JaviPi` o en otro
   repositorio Git Linux: los agentes pre-5C no reciben el puente nuevo.
2. Comprobar que Files conserva exploración y editor 5B; Changes muestra el
   estado Git de JaviPi, no el de Windows.
3. Abrir diff de un archivo Linux modificado: el diff debe estar en el
   panel **central** y Files seguir visible a la derecha.
4. Mostrar Branches (locales y remotas) y el HEAD; comprobar que no aparecen
   acciones mutantes.
5. Cambiar de agente, volver y refrescar: no deben mezclarse repositorios ni
   rutas de máquinas. Cerrar diff no debe terminar el agente.
6. Probar agente fuera de un repo: mostrar aviso, sin consulta a Git local.
7. Verificar guardado remoto 5B, cambios en index/worktree y archivo sin
   seguimiento: el árbol Changes se actualiza tras Refresh.
