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
- **Machines stays on the left**. **Files / Changes / Branches is a separate
  dock on the right**, visible by default, with its own close button and
  title-bar / Machines-footer toggles. Opening Files does not replace or
  collapse the machine navigator. Ctrl+B collapses only Machines.
- Files / Changes / Branches continues to operate on the **Windows workspace
  root**, even when the selected agent runs on JaviPi. The right dock is
  explicitly labeled `Local Windows files`, and when a remote agent is
  selected, `Local Windows files · not remote`. It is not a remote browser.
  Local file previews, edits, Git changes and branch actions stay local.
  Inserting a local path into a remote agent's prompt is blocked with a
  notification; inserting into a local agent still works.
- The workspace chip and active pane title remain in the title bar, but
  **no tab bar** is rendered. Keyboard tab shortcuts still work.
- Closing the final session leaves an empty workspace; it does not
  automatically launch another agent. Use + to explicitly create one.

## Right-side Files dock

The FileTreeView was audited: it reads directories, file previews,
working-tree status, changes and branches through native filesystem and
Git APIs in the **Windows GPUI process**. It is not backed by the
selected remote Pi agent. To prevent misleading actions, the dock is
labeled as local and never inserts a Windows path into a remote agent
prompt.

The local tree retains all current file operations, including creating,
renaming, previewing and moving files; none of these commands are sent
to Herdr. Real remote file browsing and editing would require an
independently verified, permission-aware remote filesystem transport;
this phase does not add one or silently substitute local files.

The Machines pane is independent on the left, and the local Files pane
is on the right. Each panel has its own open/close action; they can be
visible simultaneously.

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
- The **Files/Changes/Branches dock on the right** and the workspace
  selector still refer to GPUI's Windows project. They do **not** browse
  or change files on JaviPi and never represent remote Git. Only the
  agent's own composer footer displays its actual remote Git branch.

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

## Local stale addresses and remote startup latency

A GPUI exit intentionally stops local Herdr RPC agents, whereas JaviPi
agents may remain alive. An older `tN` local terminal may still be saved
in the workspace after it has exited. On restore:

- A confirmed **exited, GPUI-owned local** terminal is not reattached.
  A new local Pi runtime is started, using the existing local session
  path where available.
- An existing **live local** terminal is reattached as before.
- An unreachable, stopped, malformed or foreign **remote** terminal
  must **not** auto-launch a replacement. Its `javipi/tN` identity
  remains authoritative for recovery or explicit user action.
- An unknown/failed Herdr inspection fails closed rather than silently
  launching a process. Closing a tab remains an explicit termination.

Remote initialization has several distinct costs: inspection via
`terminal.get`, bridge `terminal.attach`/READY, and the final read-only
`get_state` probe through the PTY. During reattach, the first bytes may
be a screen repaint rather than a valid Pi stream. 5A initially retries
the idempotent probe after 1.2 seconds (then backs off to 4 seconds);
it **never** resends user messages or uses another `terminal.run`.
The safety deadlines of 90s for remote cold start, 60s for Pi4 bridge
startup, and 45s for reattach transport are unchanged.

When a remote connect takes 2+ seconds, the Windows launch terminal now
prints a diagnostic line of the form:

```text
gentle-shell-gpui: Herdr remote startup reattach=true inspect=...ms attach_or_ready=...ms rpc_probe=...ms
```

This differentiates a slow Pi/Herdr attach from a delayed PTY probe,
without printing credentials or protocol tokens. If delays persist,
collect these timings before adjusting the safe startup ceilings.

### Local/remote startup acceptance

1. Restart GPUI after local agent exit: selecting the saved local
   conversation launches a fresh local process, not an attach to a dead
   `tN`. No `Herdr RPC agent is no longer running` failure.
2. Simulate a stopped saved JaviPi agent: it must **not** automatically
   create a new `javipi/tN`.
3. Restore an existing live JaviPi agent; it retains exactly the same
   Herdr address with no duplicate instance.
4. Inspect the startup-timing line for each slow JaviPi connection and
   identify whether inspection, READY, or get_state takes the time.
5. Confirm reconnection after a long outage still works: no replayed
   mutating commands, no stale viewer events or duplicate terminals.

## Model picker: remote versus local catalogs and authentication

The model picker in each SessionView uses **that session's** Pi RPC process:

- `get_state` supplies the currently selected model.
- `get_available_models` supplies available model/provider choices.
- `set_model { provider, modelId }` changes the model in the same
  local or remote Pi process, never in a different agent.

A remote Herdr session forwards those commands to the Pi process on
JaviPi (or the selected remote machine), using **its remote credentials**.
Local/native and Herdr-local sessions use the credentials of the
Windows process. The model-picker menu now identifies the source as
`Models from This computer` or `Models from javipi`.

**Sign-in behavior:** The app's Providers screen is implemented on
Windows and operates on local authentication files. It cannot log into
a remote Gentle Shell instance. The model picker therefore offers a
local `Sign in to a provider` only for local agents. For remote agents,
it explains that provider credentials belong to the remote machine and
provides `Refresh models from remote agent`. This makes a read-only
`get_available_models` request over the existing Pi RPC process,
without starting or terminating an agent. A remote agent may still
need a deliberate restart if its running Pi process caches credentials;
the refresh button does not pretend to authenticate it.

**Remote-process safety:** A successful local provider login or a change
to Windows Settings previously called `reload_models` on all sessions,
which might restart running remote agents. It now affects only local
sessions: remote sessions are left running and retain their current
model, conversation, and Herdr terminal address. A remote catalog is
refreshed by that agent's explicit Refresh control or normal
`refresh_state` on reconnect. Stale model responses from an older
backend/reconnection generation are ignored, and old menu choices are
cleared before a new catalog is requested.

### Model source regression tests (Windows + JaviPi)

1. With different providers/auth installed on Windows versus JaviPi,
   select the local agent: the picker says `Models from This computer`
   and shows only models reported by its local Pi process.
2. Select a JaviPi agent: it says `Models from javipi` and lists the
   models reported by the remote Pi process, not a union of both.
3. Select a remote model. Verify a `set_model` RPC updates **only**
   that remote agent; switching back to the local agent keeps the
   local selection unchanged.
4. In the JaviPi picker, there must be **no** button that claims to
   sign into providers on Windows. The **Refresh models** button
   must refresh without creating a new `javipi/tN` or restarting Pi.
5. Change Windows provider credentials in Providers. JaviPi agents
   must remain alive and unchanged; their selected models and
   addresses must not be replaced.
6. During a JaviPi disconnect/reconnect, the picker must not show
   stale local models; once recovered, it should query the same
   remote process again.

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
7. Test Ctrl+T, Ctrl+Shift+T, keyboard cycling, split panes, and
   toggling the left Machines panel with Ctrl+B.
8. Confirm the right Files / Changes / Branches dock is visible separately
   from Machines and explicitly labeled as Windows-local. Test its title-bar
   toggle, right-panel close button and Machines-footer shortcut. Closing
   either dock must not collapse the other. Check re-rooting when switching
   Windows workspaces, local file preview, changes and branch actions.
   Inserting a local file path into a local agent still works; when JaviPi is
   selected, inserting that Windows-relative path must be blocked with a
   clear warning and no change to the remote prompt.
9. Restart GPUI; saved remote agents reattach exactly once and stay under the
   correct machine; no duplicate or replayed UI events.
10. Run in native local backend; the navigator remains usable and does not
    query Herdr Core.
11. Switch workspaces and verify each workspace shows its corresponding
    session list.

**Status:** source changes are in the PR branch. Windows Clippy/test/visual
acceptance have not yet been verified for this new navigator.
