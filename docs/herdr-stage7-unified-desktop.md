# Stage 7 — experiencia unificada GPUI / Herdr

**Rama de integración:** `feature/herdr-stage7-unified-desktop`  
**Pull request:** #28, borrador hasta que pasen Cargo, Clippy y las pruebas de Windows/JaviPi.  
**Base estable:** Stage 6 en `main` (`2db5566`).

## Contrato

El agente seleccionado es la identidad de ejecución de cada acción. Los comandos nunca reinterpretan una ruta POSIX como Windows ni usan Git local como fallback remoto. Los paneles sólo **toman prestada** la sesión; cerrar Files, History o Terminal no mata al agente dueño.

La UI comparte acciones y controles entre sesiones locales y Herdr, sin intentar reproducir el protocolo RPC como una terminal. Un terminal acompañante es una shell independiente creada **en la misma máquina y cwd** que el chat, no la consola RPC interna.

## 7A — Chat / Terminal

- Acción `Switch Chat / Terminal` en el selector de comandos y botón Chat/Terminal en título.
- El terminal acompañante se abre en un split cuando hay espacio; si no, en una pestaña normal.
- La asociación se guarda mediante `linked_session` y la dirección exacta de Herdr. Desde un terminal se vuelve a su chat sin reiniciar procesos.
- El origen de la petición se conserva durante la operación asíncrona; si cambia el foco, no se redefine el host.

## 7B — Agentes, preguntas, permisos y jobs

Los canales estructurados `desktop` ya soportaban permisos, preguntas, subagentes, jobs, cambios de modelo y notificaciones de manera independiente del host. Stage 7 los reutiliza, sin crear un segundo transporte.

- El panel Agents identifica máquina y estado de conexión.
- Las preguntas de UI no se retiran de la cola si la respuesta no se pudo entregar al agente original.
- Los comandos `cancel_job` y `cancel_subagent` no se emiten si ese agente está offline. Los botones de cancelación se desactivan cuando corresponde.
- Un fallback legado de cancelación informa cuando no pudo enviarse. Las respuestas pertenecen siempre a la sesión que las originó.

## 7C — Files

`herdr_fs_transfer_{read,begin,chunk,commit,abort}` son comandos privados interceptados por el puente Node en el proceso remoto. Pi no recibe `bash`, rutas de Windows ni tarjetas de herramientas.

| Operación | Límite | Protección |
| --- | --- | --- |
| Windows → JaviPi, Upload | Archivo regular 16 MiB | Carpeta elegida, no sobrescribe, temp privado, SHA-256 completo |
| JaviPi → Windows, Download | Archivo regular 16 MiB | Carpeta elegida, `create_new`, bloques 24 KiB con SHA-256 |
| JaviPi → otro agente, Send to | Archivo regular 16 MiB | Direcciones Herdr originales, ambos agentes vivos, sin archivo temporal Windows |
| Editor Linux remoto | Texto UTF-8 2 MiB | Revisión SHA-256 original, reemplazo atómico y conflicto ante cambios externos |

Los archivos `.git`, symlinks, rutas fuera de la raíz y nombres temporales internos quedan excluidos. Upload nunca pisa destinos existentes. El reemplazo de texto aplica una verificación optimista antes del rename; **no afirma ser un CAS de kernel contra procesos no cooperativos**.

## 7D — Persistencia

- `SavedPane` versión 2 agrega `remote_path`, `remote_owner` y `remote_draft`; carga archivos antiguos con valores vacíos.
- Reconstruye primero los agentes, después editores e historial, desde la dirección Herdr del propietario. Los paneles restaurados no poseen ni matan a Pi.
- Los buffers sucios se guardan con el SHA-256 **original**. Al recuperar, GPUI muestra la edición local y conserva la revisión original: si JaviPi cambió, el servidor rechaza sobrescribir por conflicto.
- El snapshot queda bajo el directorio privado de datos del usuario; en Unix el archivo temporal se crea con permisos 0600 antes de reemplazar el estado anterior. Carga máxima 32 MiB.

## 7E — Pruebas y criterios de aceptación

### Automáticas (Windows)

```powershell
git switch feature/herdr-stage7-unified-desktop
git pull --ff-only origin feature/herdr-stage7-unified-desktop
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
cargo run -p gentle-shell-gpui --locked
```

Las pruebas Node de filesystem/Git marcadas para Linux se omiten en Windows. Ejecutarlas también en CI Linux o en una copia temporal del fork en Linux: `node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs`. Jamás ejecutar pruebas destructivas sobre el `main` de JaviPi.

### Manuales (Windows y JaviPi)

1. Abrir chat local y remoto, usar Chat/Terminal desde la barra y desde Ctrl+K; comprobar `hostname` y `pwd` en la máquina correcta.
2. Crear dos agentes remotos en el mismo proyecto y alternar foco; ninguna terminal ni comando migran entre identidades.
3. Listar Agents, Jobs y permisos. Abrir pregunta de UI, desconectar remoto y contestar: la pregunta sigue pendiente y no se afirma envío.
4. Con un job vivo, desconectar y pulsar cancelar: no aparece confirmación falsa de cancelación en JaviPi.
5. Seleccionar un archivo binario de 200 KiB en Windows → Upload; descargarlo y comprobar hash SHA-256. El mismo nombre ya existente debe ser rechazado, sin clobber.
6. Con dos agentes remotos, seleccionar archivo → Send to segundo agente; verificar contenido, ruta de destino y host. Cambiar de máquina durante el envío y verificar que no se retargetea.
7. Abrir texto remoto de >128 KiB (ej. 180 KiB), editar, guardar, recargar. Editar externamente antes del guardado: debe fallar con conflicto.
8. Abrir texto remoto, dejar cambios sin guardar, cerrar GPUI, volver a abrir y reconectar el **mismo** terminal Herdr; el editor y borrador deben volver.
9. Repetir caso 8 modificando externamente el archivo durante el cierre: el borrador local reaparece, pero el servidor no acepta una revisión antigua.
10. Recuperar un panel History remoto junto a Files y Chat; ninguno puede abrir Git ni archivos de Windows. Cambiar de máquina y verificar la identidad visible.
11. Abrir un workspace antiguo (versión 1): continúa cargando paneles y sesiones locales sin campos `remote_*`.
12. Comprobar acciones locales preexistentes de Files, Changes, Branches, prompts, permisos, arrastre/redimensionado y Git: ninguna regresión.
13. Revisar archivos >16 MiB y no regulares: mensaje claro de límite, ningún volcado a la conversación.

## Límite de alcance

No modifica las capacidades exclusivas del host —cuentas locales, el navegador de Engram de Windows y la instalación de Architecture Docs— ni pretende que funcionen remotamente sin una API genuina. El objetivo de Stage 7 es paridad en **acciones del agente y experiencia compartida**; la integración futura del PR upstream #3277 sigue separada.

**Cierre formal pendiente:** hasta disponer de CI o las pruebas humanas Windows/JaviPi, este PR no se fusiona a `main`.
