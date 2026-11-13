# Review before syncing upstream gentle-shell-gpui v0.1.14

Checked 2026-11-13. **The upstream for this Rust GPUI application is**
`ismaelosuna7824/gentle-shell-gpui`, not
`Gentleman-Programming/gentle-shell-desktop` (a different project).

- Source tag: `v0.1.13` = `ca3fa1effa46f9ac6384228cc8620d647ee3f785`.
- New upstream release: `v0.1.14` =
  `7d9fad60d32f594f2805fa8c047babcc6ebc9d5d`.
- Current fork `main`: `cefcb0e44b205d4dbdc1353dc432b1bb948086e4`
  (Herdr through stage 4D, still at app version 0.1.13).
- Stage 5A PR: #10, `feature/herdr-stage5a-compact-tabs`.
- Protected snapshot of stage 5A HEAD:
  `backup/herdr-stage5a-before-upstream-0.1.14`, commit
  `9fc4f1f68fda3476f98e7703b634eff5d17abfb2`.
- Empty integration branch created from fork `main`:
  `integration/upstream-v0.1.14`; upstream changes have **not**
  been merged into it yet.

## Overlap audit

Upstream v0.1.13 → v0.1.14 modifies **47 paths**. Of those,
**21 paths** have also been modified by our fork since the common base.
This is a *file overlap count*, not a declaration that Git will produce
21 textual conflicts. A full, verified rebase has **not** been performed.

| Area | Upstream v0.1.14 | Preserve when integrating Herdr |
| --- | --- | --- |
| `session/rpc.rs` | Discard replies from replaced/dead processes | 4D reconnection/recovery, per-agent RPC, model and filesystem requests |
| `session/view/model_picker.rs` | Loading status until models arrive | Remote vs local catalog and remote-only refresh; no Windows login for JaviPi |
| `workspace/panes.rs`, `closed.rs` | Confirm closing busy agents and undo close | Never automatically spawn a replacement remote agent; preserve terminal identity and selective close |
| `workspace/render.rs`, `view.rs`, `mod.rs` | New close hover, agent activity and surge visuals | **No top tabs**; Machines left, contextual Files right, session persistence |
| `files/tree.rs`, `app/integration.rs` | Worktree actions and cross-workspace activity | Windows tree remains local; JaviPi's right tree stays remote/read-only |
| `session/events.rs`, `state.rs`, `view/composer.rs` | Nested Git repo information and activity | Remote Git branch and cwd are queried from JaviPi, never Windows |
| `Cargo.toml`, `Cargo.lock` | Version 0.1.14 | Keep `crates/herdr-client` as workspace member, preserve Pi-RPC dependencies |
| Git core, hydration, themes, settings, scroll | Local Git bugfixes, water reminder, UI improvements | Mostly additive; validate when wired into changed Workspace and app paths |

**Highest-risk specific behavior:** upstream's default `close_tab_now`
still starts a new agent after the last tab closes. Stage 5A deliberately
leaves the workspace empty to prevent unintended new local/remote
processes. Do **not** paste upstream `workspace/panes.rs` wholesale.

## Recommended integration procedure

The fork has **243 commits** from the original `v0.1.13` ancestor to
`main` (Herdr 1–4D) and another **88 stage-5A commits** on its PR branch.
Rebasing the entire 331-commit history directly onto upstream
`v0.1.14` would replay Herdr's merged history and is unnecessarily
high-risk. Prefer a two-stage integration:

1. **Integration branch from fork main** (not from PR #10):
   ```powershell
   git status --short
   git fetch origin
   git remote add upstream https://github.com/ismaelosuna7824/gentle-shell-gpui.git
   git fetch upstream --tags

   git switch --track origin/integration/upstream-v0.1.14
   git merge --no-ff --no-commit v0.1.14
   ```
   If `upstream` already exists, omit `git remote add`.
   Resolve the conflicts without losing Herdr's RPC and
   machine-aware workspace. If cancellation is necessary, use
   `git merge --abort` on this disposable integration branch.
   After resolving, `git add` only reviewed files and
   `git commit` the merge.
2. Run Clippy, Rust tests, bridge Node tests and all manual local/JaviPi
   regression checks **on the integration branch**.
   Create a separate upstream integration PR, review it and merge it to
   fork `main` only after validation.
3. **Rebase stage 5A**, and *only* stage 5A, onto the newly updated
   `origin/main`:
   ```powershell
   git fetch origin
   git switch feature/herdr-stage5a-compact-tabs
   git rebase --rebase-merges --onto origin/main cefcb0e44b205d4dbdc1353dc432b1bb948086e4
   ```
   The boundary SHA is the original stage-5A base commit, before the
   upstream integration PR; using `--onto` replays only 5A work.
   Resolve per-commit conflicts, verify, then push only with
   `git push --force-with-lease origin feature/herdr-stage5a-compact-tabs`.
   If this is too conflict-heavy, abort the rebase and instead prepare a
   reviewable squashed stage-5A integration branch from the new main.
4. Preserve the backup branch and do **not** merge PR #10 to main until
   this has compiled and passed manual remote tests.

## Required verification

```powershell
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

- With JaviPi connected, restore existing `javipi/tN` sessions without
  duplicates or restarted processes.
- Create a new remote agent with custom cwd; test **remote** read-only
  Files / Insert Path and local Windows Files/Changes/Branches separately.
- Change local providers: remote models/auth must stay isolated.
- Close one of two remote agents; preserve the other. Closing the final
  agent must leave the workspace empty, even with upstream close/undo.
- Confirm busy-agent close warnings and reopening a closed local session
  work without resurrecting/deleting a remote terminal unexpectedly.
- Verify upstream Git bugfixes, worktree actions and activity displays
  never use Windows paths for remote sessions.

**Status:** Review and backup completed. No upstream rebase, branch merge,
or upstream version bump has been executed on the fork yet.
