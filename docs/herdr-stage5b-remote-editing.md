# 5B — Remote text editing, first vertical slice

Status: **draft / pending Windows and JaviPi validation**. This is a
separate feature branch from integrated `main`; no main or upstream PR
has been changed.

## Goal

In the right Files dock, select a remote JaviPi agent, open an existing UTF-8
file under its Linux launch root, click **Edit**, change text and **Save**.
The UI uses the selected agent's *existing* Herdr/Pi RPC bridge, not Windows
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

- Browse files as in 5A, select a text file, then click **Edit**.
- **Save** is enabled only when the editor differs from the preview.
- **Cancel** abandons the in-view draft.
- Navigation or closing the Files dock while editing is refused until
  **Save** or **Cancel**. Switching agents or machines keeps the separate
  editor drafts *in memory*; returning to the same session restores them.
  Quitting GPUI is not backed by a persistent draft store, so save or copy
  unsaved text before application exit.
- On conflict, show the error and keep the unsaved editor content.
  Cancel and reload to view the newer remote file; automatic merging is
  deliberately not attempted.

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
   Open it in the remote Files dock. Check **Edit** is available.
3. Edit and **Save**; read the file directly on JaviPi and verify its
   content. No additional Herdr terminal should be created.
4. Open it again, edit in GPUI, then change the underlying file on JaviPi
   via another process. Press Save; the result must be a conflict and
   the GPUI draft must remain visible.
5. Try an external symlink, `..` path, binary file, >128 KiB content and
   nonexistent file: none may be changed.
6. Switch back to a local agent. Check Windows Files/Changes/Branches are
   unaffected.
