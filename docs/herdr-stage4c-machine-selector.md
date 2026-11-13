# Stage 4C — Per-agent machine selector in GPUI

## Prerequisite

This branch is **stacked on Stage 4B** (`feature/herdr-stage4b-remote-agents`),
which has not yet been merged into `main`. Test and review 4C independently;
do not merge it directly into `main` before 4B is validated and integrated.

## User workflow

With `GENTLE_SHELL_TERMINAL_BACKEND=herdr`:

1. Open GPUI and click the **+** tab menu.
2. Choose **Agent on a machine…**. This opens the existing **Machines** screen,
   using the same off-UI-thread Herdr discovery as the terminal manager.
3. Under **This computer** or a connected remote machine (e.g. **javipi**),
   click **New agent**. Disconnected machines have no New agent button;
   the **Connect** action is available instead.
4. GPUI creates one new agent tab on the chosen machine, without changing the
   machine of other tabs. Each agent tab shows its selected machine next to
   its title. Model discovery and shell tools execute in the agent's machine.
5. Existing tabs remain attached to their original Herdr address. Choosing a
   machine does **not** migrate or restart a running agent.

The same Machines screen remains accessible via the title-bar Machines button.

The pre-existing **Agent tab** (Ctrl+T) keeps the legacy default selection for
compatibility: local Herdr unless a global 4B machine setting was explicitly
provided. **Agent on a machine…** overrides that global default *only for that
new tab*.

**Backend `local`** keeps native GPUI/Pi behavior: the Machines selector is
hidden/disabled and no Herdr Core connection is attempted.

## Persistence

The existing workspace `SavedPane` JSON now has optional `agent_target`:

```json
{"kind":"agent","cwd":"D:/Programacion",
 "agent_target":{"kind":"remote","machine":"javipi","cwd":"/srv/workspaces/JaviPi"}}
```

For a local Herdr agent:

```json
{"kind":"agent","cwd":"D:/Programacion",
 "agent_target":{"kind":"local"}}
```

The original `herdr_terminal` field continues to store the exact allocated
terminal address, such as `javipi/t42`. When that address exists, reconnecting
to it takes precedence over the `agent_target` hint; no new process is
launched. Old workspace JSON without `agent_target` is still readable.

Older 4B remote agents are discovered **across all registered remote
machines**, not only the one named by the old
`GENTLE_SHELL_HERDR_AGENT_MACHINE` environment variable. On restart, GPUI
first checks for live remote agents, so it does not automatically create
duplicates. Explicit New agent is always allowed.

## Remote directory behavior

Herdr's target `cwd` must refer to a **Linux path on the selected machine**,
not a Windows path. The currently supported safe defaults are:

- Machine named `javipi`: `/srv/workspaces/JaviPi` (matches Stage 4B).
- Other Linux remote machines: `/`, so no nonexistent or Windows host
  workspace path is assumed.

Selecting a machine does not transfer files from Windows. A per-machine
working-directory editor and targeting remote Windows machines are outside
this first 4C increment. The configured `GENTLE_SHELL_HERDR_REMOTE_NODE`,
`GENTLE_SHELL_HERDR_REMOTE_BIN`, and
`GENTLE_SHELL_HERDR_REMOTE_PATH` overrides from 4B remain supported.

## Test from Windows (no Raspberry Pi install required)

```powershell
cd D:\Programacion\gentle-shell-gpui-herdr-core
git fetch origin feature/herdr-stage4c-agent-machine-selector
git switch -c feature/herdr-stage4c-agent-machine-selector FETCH_HEAD

cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs

$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
Remove-Item Env:GENTLE_SHELL_HERDR_AGENT_MACHINE -ErrorAction SilentlyContinue
cargo run --locked -p gentle-shell-gpui
```

Before and after starting GPUI, compare `hc ls` to check that existing JaviPi
agents are not duplicated.

## Acceptance matrix

| ID | Test | Expected |
|----|------|----------|
| C1 | Open + → Agent on a machine… | Machines screen lists local and connected remote machines |
| C2 | Start New agent on This computer | Herdr-local agent, `tN`; remote agents unaffected |
| C3 | Start New agent on javipi | Remote `javipi/tN`, tools return `aarch64`, `dietpi` |
| C4 | Two agents on different machines | Independent model lists, responses and tool execution |
| C5 | A machine is disconnected | No New agent action for it; Connect remains available |
| C6 | Restart GPUI with both tabs open | Both original Herdr addresses recovered; no new agent automatically started |
| C7 | Close one agent tab | Only that agent is terminated |
| C8 | Native backend `local` | No Herdr UI action or Herdr process |
| C9 | Force global legacy machine to javipi, choose This computer | New selected tab runs locally; legacy tabs keep configured behavior |
| C10 | Old workspace JSON without target | Opens successfully, reconnects existing terminals |
| C11 | Machine list fails | UI reports error; does not create a new remote process |
| C12 | Run Windows Rust Clippy, tests, Node tests | All checks pass on the final branch commit |

**Not yet validated:** C1–C12 require actual Windows and JaviPi execution.
Stage 4D remains responsible for automatic reconnection after a mid-turn
network interruption; 4C preserves Stage 4B's restart/reattach behavior.
