# Stage 5E.3 — Shared Changes presentation (first vertical slice)

**Status:** draft, stacked on 5E.2 (PR #18). Requires Windows build,
Clippy, tests and end-to-end GUI validation before merging.

## Goal

One GPUI presentation for the **Unstaged** and **Staged** Git change rows
regardless of whether the current workspace is local or served by Herdr.

## Implemented

- `changes_ui.rs` owns the shared `ChangeSide`, section header,
  entry row, status badge and staged/unstaged projection of
  `gentle_core::git::status::StatusEntry`.
- Local `ChangesView` uses this shared renderer and no longer maintains
  a second copy of the row/section CSS-equivalent GPUI construction.
  Local grouping, contextual actions, commit, stash and polling remain
  unchanged.
- Herdr Changes uses the **same** shared renderer. A file with staged and
  unstaged changes appears in both sections (just as with local Git).
  Per-file Stage/Unstage uses the local-style checkbox. The checked
  state indicates the Staged section.
- Remote file actions continue through the typed WorkspaceBackend with
  HEAD/branch/index and per-file revision checks. A conflicting file is
  not stageable through the checkbox.
- The section-wide Stage all / Unstage all controls are visibly disabled
  in Herdr until guarded bulk RPC commands exist. They are not silently
  wired to unguarded local Git.
- Unit tests check that the same staged/unstaged projection is used for
  remote porcelain-v1 and local porcelain-v2 normalized entries.

## Still not parity complete

Remote Changes has neither the local contextual discard/stash/push
operations nor the local directory grouping, commit summary/body and
full-screen virtualization. Branches and Files still have distinct
rendering. The next 5E.3 increments will consolidate these rather than
perpetuate parallel screens.

## Validation (Windows)

From an isolated checkout of `feature/herdr-stage5e3-shared-changes-ui`:

```powershell
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p gentle-shell-gpui --locked
```

**UI acceptance checklist:**

1. Open local Changes in a Git repository with files: staged-only,
   unstaged-only, staged+unstaged, and untracked. Verify the two
   sections, counts, badges, open diff, Stage/Unstage and commit still work.
2. Open a **new** JaviPi agent on a disposable repo with the same mix.
   Verify identical row geometry and that staged+unstaged appears in
   both sections, each with the correct checkbox state.
3. Toggle each checkbox in the remote section and verify `git status --short`
   on JaviPi. Stage/Unstage must not modify the worktree contents.
4. Make a remote HEAD/index change from a separate terminal; the
   stale remote mutation must be rejected until Refresh.
5. Open/close Changes repeatedly, navigate Files and Branches,
   reconnect, and confirm no Windows stack overflow or cross-machine
   Git action.

On JaviPi:

```bash
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

**Dependency:** PR #15 → PR #16 → PR #17 → PR #18 → this PR.
Merge only in that order, after all preceding drafts are validated.
