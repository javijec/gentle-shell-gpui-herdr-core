# Stage 5E.4 — Shared Branches rows (first vertical slice)

**Status:** draft, stacked on Stage 5E.3 (PR #19). Windows build,
Clippy, tests and UI parity regression checks are pending.

## Goal

The Branches tab must share one presentation for the same Git branch,
regardless of whether it is running in the local workspace or on a
remote Herdr agent.

## Changes

- `branches_ui.rs`: one reusable GPUI `branch_row` draws the icon,
  current-branch emphasis, branch name and trailing status for both modes.
- `RefsView::render_branch` now uses that shared row instead of
  constructing separate styling. Local checkout, context menu, new worktree
  and upstream tracking information remain intact.
- `RemoteFilesView::push_git_branch_row` renders the **same** row and
  injects its existing guarded Switch / Confirm switch button. The
  `herdr_git_switch_branch` command, clean-tree check and optimistic
  Git revision guards are unchanged.
- `BranchDisplay` only includes information available from the backend.
  The 5D Herdr bridge does not report ahead/behind or upstream-gone
  status; no values are fabricated. Tests cover this and the local
  ahead/behind preservation.
- No Windows Git action can be invoked from a Herdr branch view.

## Scope boundaries

This does **not** finish Branches or full UI parity yet. The remote bridge
still lacks the local Refs view's remotes, tags, stashes, submodules,
worktrees, filtering and most advanced actions. Those require corresponding
typed data and guarded operations before the `RefsView` itself can replace
the remote Branches panel. Remote Git's visible Switch button remains,
whereas local checkout uses click/context menu; action parity remains
an explicit follow-up milestone.

## Windows validation

From an isolated checkout of `feature/herdr-stage5e4-shared-branches-ui`:

```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p gentle-shell-gpui --locked
```

Acceptance checks:

1. Local Branches list retains the active branch marker, upstream
   gone/ahead/behind status, context menu checkout and worktree action.
2. Open a newly connected Herdr agent on JaviPi: its branch rows
   have the same icon, font, spacing, badge and active-branch marker.
3. Remote Switch requires the same two-step confirmation as 5D and refuses
   a dirty repository. Only the selected remote repository is affected.
4. The Branches panel refreshes after branch creation/checkout and does
   not crash when switching between local and remote sessions.
5. Files and Changes from 5E.3 remain functional. There is no Windows
   UI stack overflow on navigation.

On JaviPi, validate the bridge:

```bash
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

PR dependency: #15 → #16 → #17 → #18 → #19 → this PR. Only merge
after the dependent branches are validated in order.
