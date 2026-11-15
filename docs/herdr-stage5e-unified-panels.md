# Stage 5E — Full local/Herdr parity

**Status:** staged migration; 5E.1 starts from the unmerged 5D branch.
A green 5D test suite is a prerequisite for merging this work.

## Product invariant

A person can open any workspace, local or remote, and see exactly the
same tabs, controls, keyboard shortcuts, editor, error patterns and available
actions. The selected machine and location remain clearly visible to prevent
destructive actions being executed on the wrong host.

The backend handles transport, filesystem, Git and process execution. UI
never decides whether an action exists based only on the host type.

## Existing divergence (baseline)

| Feature | Local | Herdr (5D) |
| --- | --- | --- |
| Files | FileTreeView | RemoteFilesView |
| Changes | ChangesView | RemoteFilesView Git panel |
| Branches | RefsView | RemoteFilesView Git panel |
| Editor | FileView | RemoteFilesView pane |
| Diff | FileView | RemoteGitDiffView |

5D deliberately implemented only guarded Git writes. The next stages must
remove this duplication, not create an independent advanced remote UI.

## 5E.1 — First shared slice (this PR)

- **One** `FilePanelTab` enum and `tabs::tab_button` GPUI component, used
  by both local `FileTreeView` and remote `RemoteFilesView`. Both now
  display the same icon tabs, geometry, badges, states and tooltips.
- **One** status model: remote porcelain-v1 XY entries are normalized into
  `gentle_core::git::status::StatusEntry`, the same model consumed by
  local Changes (porcelain v2).
- Stage/Unstage availability, staged counts and commit gating now use the
  shared `StatusEntry::staged()/unstaged()` semantics, including conflicts.
- Existing 5D bridge commands, revision guards, sessions and remote paths
  remain unchanged. No Linux path is converted into a Windows `PathBuf`.

This is NOT full UI or action parity yet. It is an incremental architectural
migration intended to preserve 5D safety and validate shared GPUI pieces.

## Further 5E milestones

1. **5E.2** Introduce an asynchronous, typed `WorkspaceBackend` contract
   for list/read/create/rename/move/remove/write and Git operations, with
   `LocalBackend` and `HerdrBackend` adapters. Include workspace/session
   identity on every request; reject stale responses.
2. **5E.3** Reuse `FileTreeView` and `ChangesView` presentation in remote
   workspaces. The panels receive source-neutral snapshots/actions instead
   of accessing local filesystem and Git directly.
3. **5E.4** Reuse `RefsView`, `FileView` and diff presentation for both.
   Keep platform-native only where necessary; never silently omit actions.
4. **5E.5** Achieve action parity with explicitly safeguarded remote
   implementations (stash, discard, push/pull, worktrees, multi-selection,
   file management) and equivalent safety guarantees locally.
5. **5E.6** Add contract tests running the same scenarios against both
   backends plus UI parity tests and disconnection/reconnection tests.

## Validation

On Windows from the **5E branch**, not the existing 5B/5C checkout:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p gentle-shell-gpui --locked
```

Verify the local and new Herdr agent display the same tab navigation,
that Changes separates staged/unstaged states as before, that Stage and
Unstage remain guarded, that Branches still loads, and that opening Changes
does not cause a Windows stack overflow.

On JaviPi:

```bash
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

Do not merge before Windows compilation and visual checks, or modify
an existing local checkout with uncommitted work.
