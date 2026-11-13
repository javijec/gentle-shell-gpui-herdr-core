# 5A — Machines & Agents sidebar (no visible tabs)

This is the requested behavior from the supplied Herdr screenshot: a permanent
left-hand navigator grouped by machine, with each machine's agents/sessions
listed beneath it. **It replaces the top tab strip**; it is not a compact
tab treatment.

## UI

- Left sidebar: **Machines**, with **This computer** and every Herdr remote
  machine registered in Core (including disconnected machines).
- Each machine can be expanded/collapsed, shows an availability dot and a
  count of its session entries.
- Child entries represent active workspace agent sessions (and shell/file
  tabs so they remain reachable). Click one to show its existing conversation;
  hover reveals its full title and Herdr terminal address.
- A subtle active-row background replaces the old selected-tab highlight.
  A working dot marks an active turn; × closes only that session (respecting
  existing unsaved-edit confirmation).
- The **+** on a machine launches a new agent on that specific machine.
  Remote launch is disabled when the machine is reported disconnected. The
  top + creates an agent on This computer.
- Selecting a session does not spawn an agent and does not change its
  machine. Existing stable identities, saved workspaces, split-pane layout,
  per-agent routing and 4D recovery remain intact.
- The native `local` backend displays This computer without making Herdr
  requests. Herdr snapshots are fetched on a background executor every 12 s
  while Herdr mode is enabled.
- The existing Files/Changes/Branches sidebar is still accessible using the
  Files/Machines switch button in the title bar; it occupies the *same*
  sidebar slot rather than stealing additional horizontal space.
- The workspace chip and active pane title remain in the title bar, but
  **no tab bar** is rendered. Keyboard tab shortcuts still work.
- Closing the final session leaves an empty workspace; it does not
  automatically launch another agent. Use + to explicitly create one.

## Scope

This is a presentation/navigation change. The internal `Tab` / `Pane`
structures stay because existing workspace serialization, split panes,
session lifecycle and 4B/4D recovery depend on them. There is no
`workspaces.json` migration and no remote Herdr modifications.

Machine sessions are shown for the currently selected workspace; select
another workspace using the existing workspace chip to see its sessions.

## Validation matrix (Windows)

```powershell
cd D:\Programacion\gentle-shell-gpui-herdr-core
git fetch origin
git switch feature/herdr-stage5a-compact-tabs
git pull --ff-only origin feature/herdr-stage5a-compact-tabs

cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p gentle-shell-gpui
```

1. Verify **no visible top tabs**, and that the left navigation lists
   This computer and JaviPi with their existing agents beneath them.
2. Click each agent entry; ensure the correct conversation is displayed and
   the existing remote `javipi/tN` remains unchanged.
3. Use + under **This computer** and under **javipi**; confirm independent
   local/remote agents and no accidental fallback to Windows.
4. Make JaviPi unreachable; the machine should appear disconnected at the
   next snapshot and the remote + should not initiate another agent.
5. Close just one of two agents with ×; the other remains live.
6. Close the final agent: no replacement process is created.
7. Test Ctrl+T, Ctrl+Shift+T, keyboard cycling, split panes and sidebar
   collapse/expand.
8. Toggle Machines ↔ Files; confirm file tree, Changes and Branches still
   work, including inserting a file path into the active agent.
9. Restart GPUI; saved remote agents reattach exactly once and stay under the
   correct machine; no duplicate or replayed UI events.
10. Run in native local backend; the navigator remains usable and does not
    query Herdr Core.
11. Switch workspaces and verify each workspace shows its corresponding
    session list.

**Status:** source changes are in the PR branch. Windows Clippy/test/visual
acceptance have not yet been verified for this new navigator.
