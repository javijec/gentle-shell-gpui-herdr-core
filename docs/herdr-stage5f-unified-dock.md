# Stage 5F — One Files/Changes/Branches dock, local and Herdr

**Status:** Integration draft PR #23, stacked onto PR #22 (5E.6).
No separate acceptance rounds for each component. This entire integration
must pass strict Windows build/Clippy, Rust tests, bridge tests on JaviPi and
a single local/remote user acceptance cycle before merging.

## Architecture and actual reuse

The same **native GPUI ChangesView and RefsView** are now rendered for
both local and Herdr, with their execution effects routed separately:

| UI / operation | Local | Herdr |
|---|---|---|
| Right dock shell/title/file rows | `dock_ui` | **same** `dock_ui` |
| Files folder navigation | Native lazy tree | Lazy POSIX tree, same expandable-row renderer |
| Files create file/folder | Native filesystem operations | Explicit guarded `herdr_fs_create` (exclusive create; no overwrite) |
| Files read/save | Native `FileView` | Existing remote editor, same `EditorState`, SHA-256-fenced remote save |
| Changes | Native `ChangesView` | **same** `ChangesView` receiving remote `WorkTree` |
| Git Stage / Unstage | Local Git via `WorkspaceBackend` | **same** typed event/operation via Herdr guards |
| Git commit | Local native form | Same `ChangesView` container, remote-confirmed commit form |
| Branches and references | Native `RefsView` | **same** `RefsView`, typed `RefsSnapshot` from Herdr |
| Branch switch | Native Git action | RefsView event → guarded Herdr switch with second-click confirmation |
| Git diffs | Native diff pane | Remote diff RPC and same GPUI diff engine |

`RemoteFilesView` still exists as the **remote controller and central editor**.
It owns agent identity, network errors, pending drafts, the POSIX filesystem
tree and guarded RPC calls. It no longer maintains separate Changes/Branches
renderers; it embeds the native views. The local FileTreeView still owns the
native filesystem controller. Do not misdescribe this as the elimination of
all source-specific controller code.

## Safety invariants

- A Herdr action always uses the selected agent RPC, never native Windows
  `git` or local file I/O. Linux paths remain POSIX strings.
- The root is the selected agent's working directory; all remote file
  operations are validated in the backend and bridge.
- File creation is exclusive (`O_EXCL`, `O_NOFOLLOW`) and has tests for
  traversal, symlink escapes, duplicate names, and `.git`.
- Saves preserve the server-side SHA-256 revision conflict checks.
- Stage/Unstage/commit and branch switch use the existing guarded Herdr
  HEAD/branch/index status checks; remote bulk Stage all and native-only
  destructive actions are disabled pending corresponding safe RPC support.
- Remote snapshot updates are machine/session-scoped. Failures do not
  fall back to local Git or local files.
- Changed-file and branch snapshots still honor the bridge's bounded limits.
- Remote tree has a 2,000 row view cap, 32-level recursion bound, and
  bounded directory listing.

## Not yet functionally equivalent

This stage **replaces the duplicated dock presentation** but does not
implement every native command in Herdr. The following are still missing
remotely: file rename/duplicate/cut/paste/trash, ignored-file view parity,
bulk Git staging, stash apply/pop/drop, Git push/pull, advanced worktree/
submodule operations, and full native editor large-file/formatting parity.
These must gain explicit guarded RPC implementations before being enabled.
All unsupported destructive commands stay unavailable; do not run them on
Windows with a remote path.

## Single integrated validation gate

On Windows, create a separate 5F worktree; run all checks in one cycle:

```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p gentle-shell-gpui --locked
```

On JaviPi/Linux (same commit):

```bash
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

Then use **disposable** local and remote repos to verify:

1. Files tree expansion/collapse, file icons, create file and folder in
   a nested directory, duplicate-name refusal, and persistence of expanded
   folders after creation.
2. Open and edit an existing remote file; protect against concurrent write
   conflicts and preserve unsaved drafts when selecting a different agent.
3. Changes: same native grouped/listed UI, staged and unstaged entries;
   Stage/Unstage guarded and remote commit requires confirmation.
4. Branches: same `RefsView` sections, search, collapse and footer.
   Confirm branch switch twice; dirty index/worktree or stale status fails.
5. Verify existing local file, Git and editor operations are unaffected.
6. Disconnect/reconnect and rapidly change the selected machine. The dock
   must never read a remote Linux path from Windows or show stale results.
7. Confirm no Windows UI-thread stack overflow, cross-machine mutation,
   stdout buffer error or orphaned agent process.

**Merge order:** #15 → #16 → #17 → #18 → #19 → #20 → #21 → #22 → #23.
Do not merge this draft unless the full validation gate passes.
