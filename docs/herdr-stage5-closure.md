# Stage 5 — Integration checkpoint and upstream comparison

**Date:** 2026-11-15
**Status:** All Stage 5 PRs #15–#23 merged into the fork's `main`.
**Validation:** User reported the final 5F Windows app compiles, launches,
and passes functional checks. No GitHub CI statuses were published for
these PRs; do not confuse manual confirmation with an automated Linux
or Windows CI run.

## Merged work

| PR | Deliverable |
| --- | --- |
| #15 | 5C: remote Git read-only Changes / Branches |
| #16 | 5D: revision-guarded Git writes |
| #17 | 5E.1: shared Files tabs and normalized status |
| #18 | 5E.2: typed local/Herdr WorkspaceBackend |
| #19 | 5E.3: native/Herdr shared Changes presentation |
| #20 | 5E.4: shared branch row |
| #21 | 5E.5: shared Branches sections and filtering |
| #22 | 5E.6: bounded read-only Tags and Stashes |
| #23 | 5F: common dock, actual ChangesView/RefsView, virtualized Herdr file tree, guarded file and Git operations |

Files and Git snapshots remain machine-scoped; Linux paths must not be
interpreted as Windows paths. The remote controller still manages the
agent-specific transport, POSIX tree and editing state.

## Important scope distinction

**The validated Stage 5 dock integration is complete; full feature parity
with every native Git/file action is not.** Intentionally unsupported
remote features include recoverable Trash/deletion, recursive folder
copy, cross-agent transfers, Git push/pull/fetch, reset/discard, stash
apply/pop/drop and advanced worktree mutations. They require dedicated
guarded RPC APIs and explicit safety/confirmation flows; no silent
fallback to Windows local actions is acceptable.

The exact list, implemented RPCs and validation scenarios are documented
in [Stage 5F](herdr-stage5f-unified-dock.md).

## Upstream review: v0.1.14 to v0.1.19

The fork descends from
[`ismaelosuna7824/gentle-shell-gpui`](https://github.com/ismaelosuna7824/gentle-shell-gpui).
As of 2026-11-15, the latest published release inspected was **v0.1.19**
(2026-11-14). Its changelog progression:

- v0.1.15: architecture docs, file Quick Look/open in installed IDEs,
  Jobs in Agents, commit history graph, paste improvements, various
  workspace fixes
- v0.1.16: profile model search, theme scenes, device control, update
  improvements
- v0.1.17: visual/logo artwork
- v0.1.18: Gentle Desktop RPC channel for Shell 4.8+ and music player
- v0.1.19: fix reopening a workspace from a close notice

GitHub's Git comparison of **v0.1.14...v0.1.19** reported 81
upstream commits touching 149 files. Comparing this to the fork's
changes since the shared ancestor identified 28 paths changed on both
sides; this is **potential overlap, not a confirmed merge-conflict count**.

Highest attention during integration:

- `crates/app/src/features/session/{state,events,rpc,lifecycle,mod}.rs`:
  existing Herdr lifecycle, remote reconnect and Desktop RPC changes
- `crates/app/src/features/files/{mod,tree,refs,viewer}.rs`:
  Stage 5 dock and native Quick Look / Git history changes
- `crates/app/src/features/workspace/{mod,render,panes,overlays}.rs`:
  workspace restoration and pane handling
- `Cargo.toml`, `Cargo.lock`, `crates/app/Cargo.toml`:
  dependency updates requiring a regenerated lockfile and cross-platform build

**Do not merge upstream into the validated fork `main` directly.**
Create a dedicated upstream integration branch from the new main, merge
the upstream `v0.1.19` release into it, resolve overlaps, then run
Rust fmt/Clippy/tests and Windows/JaviPi UI regression before promoting
the integration to main.

## References

- [Stage 5 PR #15](https://github.com/javijec/gentle-shell-gpui-herdr-core/pull/15)
- [Stage 5 PR #23](https://github.com/javijec/gentle-shell-gpui-herdr-core/pull/23)
- [Upstream releases](https://github.com/ismaelosuna7824/gentle-shell-gpui/releases)
- [v0.1.14...v0.1.19 upstream comparison](https://github.com/ismaelosuna7824/gentle-shell-gpui/compare/v0.1.14...v0.1.19)
