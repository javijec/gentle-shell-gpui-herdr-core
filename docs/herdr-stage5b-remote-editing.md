# 5B — Remote text editing, first vertical slice

Status: **draft / pending Windows and JaviPi validation**. This is a
separate feature branch from integrated `main`; no main or upstream PR
has been changed.

## Goal

In the right Files dock, select a remote JaviPi agent and click an existing
UTF-8 file under its Linux launch root. It opens in the **central workspace
editor**, beside its owning agent where there is room; the right Files dock
stays visible as the Linux directory explorer. Edit and **Save** in the
central pane. The UI uses that agent's *existing* Herdr/Pi RPC bridge, not Windows
filesystem APIs, SSH, a new terminal, or a new agent. Local Windows editing
keeps its existing implementation.

**Not included yet:** creating/deleting/renaming remote files, writing
arbitrary binary files, remote Git mutation, editors for files above 128 KiB,
or managing unsaved drafts across a full app restart.

## Transport protocol

The bridge already intercepted `herdr_fs_list` and `herdr_fs_read`.
It now supports a third private request:

```json
{
  "type": "herdr_fs_write",
  "id": "request-id",
  "root": "/srv/workspaces/JaviPi",
  "path": "/srv/workspaces/JaviPi/notes.txt",
  "content": "Updated UTF-8 text",
  "expectedRevision": "<64 lowercase hex SHA-256 chars from herdr_fs_read>"
}
```

`herdr_fs_read` returns `data.revision` as SHA-256 of the original bytes.
`herdr_fs_write` replies with the new revision and size on success.

### Write safeguards

- Explicit action only: opening/browsing/previewing never writes.
- `root` must resolve to that agent process's working directory.
  Agents launched at filesystem root (`/`) remain read-only; launch the
  agent inside a specific Linux project to enable editing.
- Path must be an absolute Linux path within root both lexically and after
  resolving symlinks. Final symlinks, missing files, nonregular files and
  multiply hardlinked files cannot be written.
- Only valid UTF-8 text of at most 128 KiB, with no NULs.
  On Linux the file must be owned by the agent's user; the bridge will
  not replace another user's file or a hardlinked inode.
- The old revision is mandatory. Different bytes mean a **conflict**, not an
  overwrite. The editor keeps the draft on rejection. Writes from the same
  bridge are serialized by canonical file path.
- Bridge writes to a private O_EXCL temporary file beside the original,
  copies its permission bits, flushes it, rechecks identity/metadata/content
  and renames it over the destination. Temp files are deleted on failed writes.
  Large payloads are sent on the background executor, not the GPUI thread.
- Same-bridge writes to the same path are serialized. The check is
  optimistic concurrency, not an OS-level atomic compare-and-swap with
  unrelated processes: a tiny race with an uncooperative external editor is
  still possible between the final validation and rename.
- Paths stay scoped to the existing agent. An old running bridge without
  the revision/write command stays in read-only mode.
- No shell commands, user transcript pollution, remote agent restart, or
  cross-machine/local path fallback.

### Interface

The **same source editor** now powers both backends: `FileView` (Windows)
and `RemoteFilesView` (JaviPi) construct `gpui_kit::EditorState` using
`files::source_editor`. Both render `Editor` with syntax highlighting,
line numbers, matching font/size, search, selection and undo/redo. The
editing engine is independent of storage: `FileView` retains local
filesystem saves and polling; remote Files continues to use revision-checked
`herdr_fs_write`, never Windows paths.

Remote file clicks now create **real central workspace file panes** with
normal split/close/unsaved-check behavior; the right dock remains the
directory browser. A remote file pane **borrows** its original agent session:
closing a file never closes that agent. The file path remains an absolute
Linux string and is never converted into a local Windows FileView path.
Remote Markdown files also use the shared `TextViewState` markdown renderer
and the same **Edit / Split / Preview** mode selector as local files; local
Git decorations, relative Markdown image assets and operating-system
"Open with default app" actions remain deliberately local-only.

Remote editor panes are currently **ephemeral across app restart**:
they are not serialized as fake Windows paths in `workspaces.json`.
The agent's session and Herdr address are still persisted normally. Closing
a dirty remote editor invokes the workspace's unsaved-edit confirmation.

- Browse Linux folders on the right; click a remote text file to open it
  in the large central file pane. The right directory tree stays available.
- `Ctrl+S` on Windows/Linux (`⌘S` on macOS) invokes the remote save.
  The editor is locked during an in-flight save to avoid losing new keystrokes.
- **Save** is enabled only when the editor differs from the loaded content.
- Closing a dirty remote file pane asks for confirmation, just like a local
  file pane. Opening another file must not silently replace an unsaved pane.
- Switching agents or machines preserves central editor panes in their
  original tab. Quitting GPUI is not backed by a persistent remote draft
  store, so save or copy unsaved text before application exit.
- On conflict, show the error and keep the unsaved editor content.
  Close and reopen the file to view the newer remote version after saving
  or copying your draft elsewhere; automatic merging is deliberately not
  attempted.

## Test plan

### Automated

```powershell
git switch feature/herdr-stage5b-remote-editing
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

The filesystem-writing Node tests use Linux/POSIX temporary paths and are
skipped on Windows. Validate them on a Linux CI runner or separate Linux
development environment before enabling this in production.

### JaviPi smoke test (use a disposable file)

1. From Machines → javipi, start a **new** agent in a temporary project.
   A pre-5B bridge cannot be upgraded in place; do not kill old agents.
2. Create a disposable UTF-8 file on JaviPi by your normal workflow.
   Click the file in the Remote Files dock. Verify it opens in a **central
   editor** while the right Files dock continues listing Linux directories.
3. Check line numbers and syntax highlighting on a disposable `prueba.rs`
   or other source file with a recognized extension. Test editor search and
   undo/redo. Edit and save using **Ctrl+S**, then again using **Save**;
   read the file directly on JaviPi and verify its content. No additional
   Herdr terminal should be created.
4. Open a disposable `.md` file and verify Edit/Split/Preview modes,
   including live split-mode preview. Never resolve remote Markdown assets
   through local Windows paths.
5. Open the file again, edit in GPUI, then change it on JaviPi
   via another process. Press Save; the result must be a conflict and
   the GPUI draft must remain visible.
6. Try an external symlink, `..` path, binary file, >128 KiB content and
   nonexistent file: none may be changed.
7. Switch back to a local agent. Check Windows Files/Changes/Branches are
   unaffected. Verify closing only the remote file leaves its agent alive,
   and dirty-file close warnings appear as in local editing.
8. Restart GPUI: the Herdr agent should restore; central remote file panes
   are not yet persisted. Reopen them from the right Linux Files dock.
