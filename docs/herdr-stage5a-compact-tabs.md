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
  hover reveals its full title and Herdr terminal address. Double-click an
  agent to rename it inline (Enter or blur saves; Escape cancels), even when a
  split-pane file happens to hold keyboard focus.
- A subtle active-row background replaces the old selected-tab highlight.
  A working dot marks an active turn; × closes only that session (respecting
  existing unsaved-edit confirmation).
- The **+** on a local machine creates a local agent. The **+** on a
  connected remote machine now opens a directory picker **before** launching
  anything. Remote launch is unavailable when the machine is disconnected.
  The picker supports an editable absolute Linux path and shortcuts from
  Herdr's known terminal directories. Cancel never starts a process. The
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

## Directories: local workspace vs remote agent

The workspace root is a **Windows local path**, used by GPUI to manage
project files and tab persistence. It is **not** passed as the launch CWD of
a remote agent.

The current per-agent Herdr defaults are:

- New `This computer` agent: active Windows workspace directory.
- New `javipi` agent: `/srv/workspaces/JaviPi`.
- New agent on another registered Linux machine: `/`.

`AgentTarget::Remote { machine, cwd }` is stored per agent, and
`terminal.run` receives its Linux `cwd`. This is the initial working
directory, not necessarily the current directory after a later `cd` or
session change. Existing remote jobs reattach to their original terminal
and **must not be recreated just to update the displayed folder**.

The navigator now shows the launch/runtime-directory metadata beneath each
agent name. When a Herdr `terminal.list` snapshot reports the terminal's
CWD, that directory takes precedence; otherwise a new agent's persisted
target CWD is shown. For legacy restored remote addresses without a stored
target or a terminal metadata result, the UI shows `Remote cwd unavailable`
rather than a misleading Windows path. The title bar shows
`machine · directory`. In native `local` mode, both titles and paths
correctly refer to the Windows process even if a prior saved pane has a
remote target.

To independently check a remote agent's *current* directory, ask that
specific agent to run `pwd` in its own shell. A new SSH session's `pwd`
is not evidence of the agent's CWD.

### New remote directory picker

Choose **Machines → javipi → +**, enter an existing remote path such as
`/srv/workspaces/PoeTradePlus2`, then click **Create agent**. The older
Machines overlay's **New agent** action uses the same picker.

The default remains `/srv/workspaces/JaviPi`, but this is not mandatory.
Quick choices include the default path, `/srv/workspaces`, root, and
previously reported CWDs of terminals belonging to **that same machine**.
This is **not a live remote filesystem browser**: a custom path can be typed
even when not present in the list. Its existence is checked by the remote
launch; if it does not exist, the launch should fail without a local
fallback. The picker validates Linux absolute paths and rejects Windows
paths, traversal, control characters and invalid machine names before
creating the agent pane.

Every created agent persists its own `AgentTarget::Remote { machine, cwd }`
in the workspace; switching to another agent or restarting GPUI does not
overwrite it. An already-running remote process is never migrated by the
picker. No global `GENTLE_SHELL_HERDR_REMOTE_CWD` override is needed.

### Runtime-directory regressions

- Check new JaviPi agents show `/srv/workspaces/JaviPi`, **not**
  `D:\\Programacion\\gentle-shell-gpui-herdr-core`.
- Check local agents show their Windows workspace path.
- Restore a legacy JaviPi terminal: it should show metadata CWD if known,
  otherwise `Remote cwd unavailable`, never a fabricated local CWD.
- Switch to native `local` backend with a saved remote target: the running
  local session and its labels must show Windows, not JaviPi.
- Check the title bar label changes when selecting local vs JaviPi agents.

## Remote agent composer footer

The footer **folder** and **Git branch** indicators are agent-scoped, not
workspace-scoped. In previous code both read Windows `SessionView.cwd`,
showing the GPUI source repository under a JaviPi agent.

- A native/local agent still uses the ordinary Windows folder dialog and
  local Git snapshot.
- A remote agent displays its saved **Linux** launch directory (or
  `remote cwd unknown` for a legacy session without target metadata).
  Clicking the footer folder opens the **remote Linux directory picker**
  for that machine. Confirming starts a *new, separate remote agent* in
  the chosen project; the original conversation/process is **not**
  restarted, moved, killed or recreated. Cancel has no side effects.
  The picker does not browse remote files in the existing agent.
- A remote agent's Git label is read from that agent over Pi RPC using
  `bash` with the read-only command
  `git symbolic-ref --quiet --short HEAD || git rev-parse --short HEAD`.
  The result is not inserted into chat context; failure or a non-Git
  directory hides the branch rather than showing Windows Git.
- The remote Git status/dirty-file count is not inferred from the Windows
  checkout. Only the verified remote branch is displayed. The branch
  is rechecked after reconnect and after regular Git refresh triggers.
  Stale responses are discarded by generation and remote session guard.
- The **Files/Changes/Branches** sidebar and workspace chip still refer
  to GPUI's local workspace; they are not a remote file explorer and
  must not claim to represent remote Git.

### Composer footer regression tests

1. Local agent: footer folder opens the Windows folder dialog; Git shows
   the local workspace's current branch.
2. New JaviPi agent with `/srv/workspaces/PoeTradePlus2`: footer shows
   that Linux path, not `D:\\Programacion\\gentle-shell-gpui-herdr-core`.
   Git shows the real remote repository branch after Pi RPC is ready.
3. Click the remote folder: the Linux directory chooser opens. Cancel
   leaves the existing agent intact; confirm creates a separate agent
   under JaviPi in the chosen directory, with a new unique Herdr address.
4. Use a remote directory that is not a Git repository: no branch is shown,
   and the Windows `feature/herdr-stage5a-compact-tabs` branch never leaks
   into that remote footer.
5. Reconnect remote agent after network loss: remote branch reappears from
   a fresh RPC query; no second Pi runtime is launched.
6. Restore a legacy 4B address with no saved target: the footer must say
   `remote cwd unknown`, never show Windows as its execution folder.

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
   the existing remote `javipi/tN` remains unchanged. Double-click an agent
   to rename it inline; verify Enter saves, Escape cancels, and a split-pane
   agent can still be renamed while its file pane has focus.
3. Use + under **This computer** and under **javipi**. For JaviPi select
   `/srv/workspaces/PoeTradePlus2`, confirm the agent's `pwd` and
   the displayed path both match, and verify it does not inherit Windows CWD.
   Open another JaviPi agent with the default `/srv/workspaces/JaviPi`.
   Both agents should retain independent paths and sessions.
   Confirm Cancel does not create a terminal, and invalid or relative Linux
   paths are rejected without spawning an agent.
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
