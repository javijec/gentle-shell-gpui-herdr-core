# Stage 5E.6 — Remote Git tags and stashes (read-only)

**Status:** draft. Stacked on Stage 5E.5 (PR #21). The latest Windows
build, full Rust tests, JaviPi bridge tests and visual checks must pass
before merge.

## Motivation

5E.5 standardized Branches and Remotes section headers and filtering.
This slice extends Herdr's *bounded, read-only Git snapshot* to more
reference types instead of inventing additional remote-only UI.

## Scope

- `crates/pi-rpc/src/herdr_remote_bridge.cjs` adds fixed `git
  for-each-ref --format=%(refname:short) refs/tags` and `git stash list
  --format=%gd%x00%gs` queries, with 120-record caps. Both run through
  the existing sanitized, no-shell Git transport and never invoke the
  agent's Pi RPC.
- The snapshot now has optional `tags`, `stashes`,
  `tagsTruncated`, `stashesTruncated`, and retains the same guarded
  Git-write identity, HEAD, branch and index checks.
- `remote.rs` parses the new records with bounded shape validation.
  Existing running agents that do not include these fields remain
  compatible and provide empty optional collections.
- The remote Branches panel uses the same `RefSection`, search,
  collapsible headers and generic ref-row component as the native
  `RefsView`, including Tags and Stashes.
- The native `RefsView` also consumes the extracted `ref_row`
  component, avoiding a second separate generic-row renderer.
- Remote stash actions remain **read-only**: no apply/pop/drop/edit.
  No new branch or stash writing commands were added.

## Not yet implemented

A single `RefsView` container for all backends, remote worktree
and submodule metadata, full remote tracking/upstream information,
and full parity with local Git stash/branch/worktree actions. UI
parity does not mean safe execution parity until the bridge implements
appropriate guarded commands. Truncated datasets are explicitly
flagged and filtered only within the returned snapshot.

## Validation

Windows, in a separate worktree of
`feature/herdr-stage5e6-remote-refs-readonly`:

```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p gentle-shell-gpui --locked
```

Bridge tests (can also run on Windows where supported; several use
POSIX-only temporary repositories):

```bash
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

Manual GUI acceptance:

1. On JaviPi use a **disposable Git repository** with a tag, an
   existing stash, at least two local branches and one remote ref.
2. Start a **new** remote agent from this version of Gentle Shell.
   The Branches panel should show Branches, Remotes, Tags and Stashes
   groups, with shared row styling and independent collapses.
3. Search for a tag name, stash name and stash message. The matching
   group and count should update; empty groups should disappear.
4. Verify that clicking a tag or stash does **not** modify Git or
   open a Windows path. Existing Stage/Unstage and guarded branch
   switch functionality must remain unchanged.
5. Open native local Branches and verify tags/stashes still render,
   and that native context menu operations still work.
6. Test rapidly changing tabs and agents. No stack overflow on Windows.
7. Ensure a repository with more than 120 tags is marked truncated
   rather than raising a stdout maxBuffer error.

## Merge prerequisites

PR #15 → #16 → #17 → #18 → #19 → #20 → #21 → this PR.
Keep every PR in draft until the relevant checks are confirmed and
merge them in base-first order. Do not modify older worktrees with
uncommitted changes.
