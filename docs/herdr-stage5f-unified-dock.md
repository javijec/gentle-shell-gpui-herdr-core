# Stage 5F — Shared local/Herdr Files, Changes and Branches dock

**Status: integration draft PR #23**, based on PR #22. The original 5F
dock was reported to compile and launch on Windows. The subsequent
file/Git parity commits in this branch still require a fresh integrated
build and real JaviPi test run.

## Shared UI / backend boundary

- **Files:** same file-row renderer, 40-pixel header, lazy folder tree,
  file icons and GPUI virtualized list as the local browser. Herdr keeps
  POSIX string paths. Keyboard arrows, Enter, F2 and isolated file
  clipboard keys work inside the tree without taking over the editor.
- **Changes:** the actual native `ChangesView` consumes a typed remote
  `WorkTree`. Stage, Unstage, bulk Stage all/Unstage all and confirmed
  Commit emit Herdr-only events. The normal local Git code never runs
  against a JaviPi path.
- **Branches:** the actual native `RefsView` consumes Herdr
  `RefsSnapshot`, including Branches, Remotes, Tags, Stashes and
  Submodules. A separate POSIX-string worktree metadata model supplies
  **read-only** worktree rows to the same view without converting Linux
  paths to Windows `PathBuf`. Remote branch switching remains confirmed
  and revision guarded.
- **Editor/Diff:** existing source editor and diff GPUI primitives,
  Markdown edit/split/preview and remote revision-checked saves are
  preserved. Session identity remains bound to the selected agent.
- **Controller:** `RemoteFilesView` is still the session-specific
  controller, remote filesystem tree and central remote editor. Its
  independent Changes and Branches renderers were removed. The local
  `FileTreeView` still owns native filesystem operations; views and
  backend dispatch are shared, not every controller method.

## Actual remote filesystem capabilities

| Function | Herdr implementation |
| --- | --- |
| List/expand folders | `herdr_fs_list` via bounded RPC, lazy per folder |
| Read/edit/save text | Existing SHA-256 compare-before-replace save, 128 KiB limit |
| Create file/folder | `herdr_fs_create` with exclusive no-clobber creation |
| Rename file/folder | `herdr_fs_entry: rename`, no destination replacement |
| Duplicate file | `herdr_fs_entry: duplicate`, exclusive 16 MiB copy cap |
| Cut/paste | `herdr_fs_entry: move`, within one remote agent/project |
| Copy/paste | `herdr_fs_entry: copy`, no replacement and duplicate name fallback |
| Git single Stage/Unstage | Guarded per-file Git request |
| Git bulk Stage all/Unstage all | Explicit fixed Git verbs and literal paths; <=80 reviewed status records, reject stale HEAD/index/status, truncated status, symlinks and unusual file kinds |
| Git commit/create branch/switch | Existing HEAD/index guard and second explicit confirmation |

All filesystem requests have exact command interception in the Node
Herdr bridge: they never forward to the Pi agent's command handler.
Create, write and entry mutations are rejected inside `.git` and the
bridge's private temporary-file paths. The bridge validates its actual
working directory, lexical/root/real paths and allowed file kinds.
Moves use exclusive linking for regular files and reserved destinations
for directories; a failed operation is reported rather than silently
overwriting a destination.

The worktree snapshot and submodule status are bounded reads only. Git
refs also carry real upstream, ahead/behind and subject information
where available. Older running bridges lacking these optional fields
continue to produce a compatible (reduced) UI; use a **new remote
agent** to activate new RPC commands.

## Still intentionally unavailable remotely

- Permanent delete, system trash or a recoverable Trash implementation:
  requires a defined safe Linux trash destination and confirmation.
- Recursive directory duplication, file operations across different
  agents/disks, native OS reveal/open-with-default-app integration.
- Git push/pull/fetch, force checkout, discard/reset, stash apply/pop/
  drop and advanced worktree creation/removal/checkout: each needs
  dedicated scope, guards and explicit confirmation.
- Some native editor high-volume/large-binary actions and fully identical
  Git ignored-file toggling/context menus.
- File tree and remote controller are not yet literally the same Rust
  `FileTreeView` type, but share the dock chrome and row renderer and
  do not have parallel Git/Changes presentations.

**Do not enable unsupported actions by accidentally running native
Windows Git or file APIs against a Linux path.** They must remain
unavailable until they have explicit remote implementations.

## One integrated validation cycle

Windows (from a separate 5F worktree):

```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p gentle-shell-gpui --locked
```

Linux / JaviPi with a disposable Git repository and a **new** agent:

```bash
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

Manual acceptance (one combined pass):

1. Local files/Changes/Branches remain unaffected; no Windows debug
   stack overflow on remote trees with many expanded folders.
2. On JaviPi create file/folder, rename file and directory, duplicate
   a file, copy/paste and cut/paste between two folders; refuse
   duplicates, outside-root moves and `.git` mutations.
3. Arrow/Enter/F2/Ctrl+C/Ctrl+X/Ctrl+V shortcuts operate in the remote
   tree, without hijacking the remote source editor's keyboard.
4. Stage/Unstage one file, Stage all/Unstage all a complete reviewable
   status, confirmed Commit and Branch switch; reject stale snapshots,
   unsupported kinds, symlinks and altered HEAD.
5. Branches filter/sections should show actual metadata, tags,
   stashes and read-only worktrees/submodules when present.
6. Switch remote agents rapidly, disconnect/reconnect and confirm that
   no work or preview leaks across machines and no orphan Pi process
   is created.

**Merge order:** PRs #15 → #16 → #17 → #18 → #19 → #20 → #21 → #22 → #23.
Keep #23 draft until the integrated validation succeeds.
