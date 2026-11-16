# Stage 8D — Shutdown lifecycle and final local/Herdr parity

**Base:** `main` after upstream v0.1.21 and Stage 8C. The design goal is that a Windows or remote-Linux project exposes the same supported UI actions while backend-specific operations remain isolated and safe.

## 8D implementation: process lifetime

An observed Windows exit left `hc.exe serve`, `hc.exe hold`, a local `gentle-shell-gpui.exe --herdr-agent-bridge`, and native `node.exe --mode rpc` descendants running. These are **not all the same kind of process**:

- `hc.exe serve` is the Herdr Core service and may legitimately outlive the desktop.
- An active remote `machine/terminal` address is deliberately retained across app exit for reconnection; closing a **tab** is still an explicit stop.
- A local Herdr agent (address without a machine prefix), a local GPUI bridge, and a native local RPC agent **must be terminated on desktop exit**.

Previously the GPUI app-quit handler waited a fixed **100 ms**, while native RPC shutdown may allow a **3-second graceful period** and Herdr `terminal.kill` runs on a worker. When the GUI exited, those workers could be cut off.

In 8D:
1. Native `PiProcess` exposes a shutdown-completion signal set only when its worker has reaped the child or the child was already gone.
2. Herdr-owned local agent `terminal.kill` signals completion when the worker returns, with failures logged to stderr for diagnosis.
3. On normal app quit, the UI waits **only for processes that should stop**, checking completion every 50 ms and giving up after 4 seconds. This avoids a fixed multi-second delay when everything exits quickly.
4. Remote Herdr agents continue to use `park_for_app_quit()`; they are excluded from the completion barrier and **never killed just because the window closes**.
5. A platform test spawns a real native child and verifies that the new completion signal follows actual process reaping.

A shutdown timeout reports a diagnostic but cannot guarantee cleanup of a process already orphaned by another launcher. Do not kill the global Herdr Core service or unrelated Node processes as a workaround.

## Functional parity acceptance checklist

Use a **throwaway repository** on both Windows and JaviPi for all destructive operations.

| Area | Windows local and JaviPi remote acceptance |
| --- | --- |
| Agent creation | Create each from Machines/Workspace; verify correct selected machine, cwd and reused session |
| Persistence | Close desktop with a remote agent running, reopen, reattach to the same Herdr address and conversation |
| Local cleanup | Close desktop with one native agent, one Herdr-local agent and an idle remote agent; verify native Node and local `--herdr-agent-bridge` disappear without stopping `hc.exe serve` or the JaviPi agent |
| Individual tab close | Close a local or remote **agent tab**; ensure exactly that owned terminal stops while unrelated agents remain alive |
| Files | Open, edit, rename, copy/move, create and Trash/Undo; confirm Linux paths and no accidental writes on Windows |
| Git Changes | Diff, stage/unstage, discard with confirmation, Commit summary+description, Amend, Write it, Commit & Push |
| Branches/History | Switch branch safely, stash, open History, create/open remote worktree with a new agent on the same machine |
| Reconnection | Disconnect JaviPi during a read, reconnect, verify machine and revision fencing remain intact |
| Transfer | Transfer an actual binary file both directions, compare bytes/checksum and ensure no overwrite without guards |
| Edge cases | No upstream, failed SSH/HTTPS credentials, stale HEAD/index, symlinks and intentionally unsupported remote capabilities must fail clearly |

### Windows commands

Run from `feature/herdr-stage8d-parity-lifecycle`:

```powershell
git fetch origin
git switch feature/herdr-stage8d-parity-lifecycle
git pull --ff-only origin feature/herdr-stage8d-parity-lifecycle

cargo fmt --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs

$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
cargo run --locked -- .
```

After closing the **desktop window**, inspect exact command lines (do not kill processes blindly):

```powershell
Get-CimInstance Win32_Process |
    Where-Object {
        $_.Name -in @('hc.exe','gentle-shell-gpui.exe') -or
        ($_.Name -eq 'node.exe' -and $_.CommandLine -match 'gentle-shell|gentle-pi')
    } |
    Select-Object ProcessId, ParentProcessId, Name, CommandLine |
    Format-List
```

Check machine-qualified addresses before concluding a `hc.exe hold` or bridge should die. Note that unrelated standalone instances of Gentle Shell may use the same Node runtime.

### JaviPi/Linux

In an isolated checkout/repo on the target machine, run:

```sh
node --test crates/pi-rpc/tests/herdr_remote_bridge.test.cjs
```

All Linux-only bridge tests that are SKIP on Windows should run here; do not point destructive tests at the real `/srv/workspaces/JaviPi` data.

## Explicit limits

- `hc.exe serve` is an external Herdr service, not embedded in the desktop. Its continued existence after window close is expected.
- Operating-system and desktop-specific Settings/Providers/Setup actions should remain local rather than pretend to edit Linux configuration files. Remote Profiles use supported RPC or report unsupported.
- v0.1.21 animated themes are app-UI behavior; they should not require a running JaviPi agent.
- This implementation and audit do **not** independently establish Windows/JaviPi runtime parity. Cargo, Clippy and real exit/reconnect/transfer tests must be confirmed before merging this 8D PR.
