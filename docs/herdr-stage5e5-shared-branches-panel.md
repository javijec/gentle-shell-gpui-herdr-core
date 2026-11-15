# Stage 5E.5 — Shared Branches sections and filter

**Status:** implementation draft on top of 5E.4 (PR #20).
Windows compilation, Clippy and manual UI validation are still required.

## Implemented

- Extend `branches_ui.rs` with one `RefSection` enumeration for
  Branches, Remotes, Tags, Stashes, Worktrees and Submodules.
- A **single** `ref_section_header` draws collapsible sections with the
  same dimensions, typography, chevrons and count in native and Herdr.
- A **single** `filter_field` draws the filter input and optional busy
  spinner in local and remote Branches.
- A **single** case-insensitive `matches_filter` governs both views.
  `branch_matches` applies the local-versus-remotes grouping to Herdr's
  existing branch snapshot and has regression tests.
- Native `RefsView` keeps its existing branch/remotes/tags/stashes/
  worktrees/submodules data, actions, worktree shortcuts and virtualization.
- The remote Branches panel now separates **Branches** (local) and
  **Remotes**, supports interactive collapse/expand and text filtering,
  and preserves original snapshot indexes for guarded branch switching.
- Git writes and reads are **unchanged**: no implicit fallback to
  native Windows Git; remote still checks the agent/repository/HEAD/index.

## Boundaries

This migration shares Branches navigation/presentation, **not** full
feature/action parity. The 5D bridge only provides local and remote branch
names plus the active-branch flag. It does not yet report tags, stashes,
worktrees, submodules, ahead/behind or the metadata needed for all native
RefsView operations. This stage does not fabricate those sections or enable
unsupported remote actions. A truncated remote snapshot remains explicitly
marked and filtering operates on loaded data only.

Next milestones should extend the backend protocol and present actual
`RefsSnapshot` models to a single refs view, then converge the Files tree
and editor/diff without parallel remote-only UI.

## Validation (Windows)

Create a separate 5E.5 worktree and run:

```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p gentle-shell-gpui --locked
```

**Manual UI checks**

1. In local Branches verify filtering by partial name (case-insensitive),
   all six sections, collapse/expand, active branch marker, checkout,
   context menus and worktree creation.
2. On a **new** Herdr/JaviPi agent, verify that remote Branches shows
   **Branches** and **Remotes** as separate groups, that both filter by
   partial name and can collapse independently, and that counts match.
3. On a disposable remote Git repo, confirm the Switch → Confirm switch
   flow works for a **local** branch but isn't offered for a remote ref.
   Dirty tree and stale snapshot guards must still reject unsafe switches.
4. Switch rapidly between local, multiple remote agents, Files, Changes
   and Branches. Filter text must remain scoped to its own view. No stale
   data or Windows stack overflow.
5. Verify that Changes Stage/Unstage, file read/save and tabs still work.

JaviPi bridge regression:

```bash
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

**Dependency:** PR #15 → #16 → #17 → #18 → #19 → #20 → 5E.5.
Retain as draft until the validations are confirmed.
