# Herdr Stage 5E.2 — Typed WorkspaceBackend vertical slice

**State:** implementation in a stacked draft PR, awaiting Windows build, Clippy, tests, and UI validation. Parent: 5E.1 / PR #17.

## Goal

One location-aware execution contract, not duplicated UI logic or implicit
fallbacks. The `WorkspaceBackend` in
`crates/app/src/features/files/backend.rs` is an incremental typed API,
implemented by `Local` and `Herdr` variants. Both use the same
asynchronous result types; native blocking work is dispatched by its UI
callers onto the GPUI background executor.

## What is wired in this vertical slice

| Operation | Local | Herdr |
| --- | --- | --- |
| List a folder | `FileTreeView` through the backend | `RemoteFilesView` through the backend |
| Read UTF-8 text (128 KiB) | Backend adapter implemented; local `FileView` still uses its current path | `RemoteFilesView` through the backend |
| Save existing text with expected SHA-256 revision | Backend adapter implemented; local editor migration pending | `RemoteFilesView` through the backend, server atomic save retained |
| Git Stage/Unstage | `ChangesView` uses typed `GitFileMutation` | `RemoteFilesView` uses the same operation and a required Herdr revision guard |

The backend owns an explicit **native path** or **remote Linux path**;
crossing them is rejected. For Herdr, the selected agent RPC process, root
directory, repository top level, HEAD, branch, index fingerprint and selected
file status remain part of every guarded Git operation. The bridge still
verifies them independently; the backend is not a substitute for server
authorization. Local directory traversal preserves the existing native
file-tree semantics, including local symlinked folders.

List behavior is **not fully identical yet**: the native tree honors
ignore patterns and the 5D bridge shows at most 500 remote entries, including
ignored ones. This divergence is tracked for the shared tree migration.

## Not implemented in 5E.2

- Replacing the remote `RemoteFilesView` visual tree with `FileTreeView`
- Replacing remote Git visual views with the local `ChangesView` / `RefsView`
- Routing local `FileView` reads/saves through this backend
- Typed rename/move/delete/create, advanced Git operations, stash, push,
  branch switching, or terminal lifecycle APIs
- Uniform handling for non-UTF8 local native file names
- Complete local revision-safe atomic replacement for native writes

The remaining 5D commands intentionally continue using the guarded bridge;
do not mistake a common interface for feature parity that has not landed.

## Validation

From an isolated Windows checkout of the 5E.2 branch:

```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p gentle-shell-gpui --locked
```

Visual regression checklist:

1. Open local Files/Changes/Branches, expand folders, including files
   affected by `.gitignore`; verify existing sorting and staging.
2. Open a **new** Herdr agent on JaviPi and list/read a remote file.
3. Save a remote UTF-8 file and verify SHA-256 revision conflict checks
   still refuse to overwrite concurrent edits.
4. Stage and Unstage a single file in local and in a disposable JaviPi
   Git repository. Verify each only affects its selected machine.
5. Verify remote Changes and Branches load without Windows stack overflow.
6. Disconnect a remote session during a query: display an error, never
   fall back to reading or writing Windows files.

On JaviPi, run `node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs`
from the 5E.2 checkout. Git test repositories are temporary.

The PR stays in draft until all compilation/tests and the manual checks
are confirmed. Subsequent milestones converge the visual components and
file-management actions rather than adding another parallel remote UI.
