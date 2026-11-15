# Stage 8A — Herdr machine management

This stage extends the **Manage machines** modal without turning it into a session list.

## Real Herdr Core API

The desktop delegates machine connections to the **local Herdr Core**. Supported RPC calls:

- `machine.list`: registered machine profiles and their actual state.
- `machine.add`: an SSH `target` **or** a custom duplex `via` command that can run a shell line on the destination. No terminal is created by the desktop.
- `machine.connect`: ask the existing Core to redial a registered machine now.
- `machine.rm`: forget a registered machine. **Does not kill remote programs or terminals.**

Herdr Core itself reconnects after connection failures. For `needs_login`, it backs off (roughly 2, 5, then 10 minutes); the desktop does not bypass its authentication rules.

The currently published Herdr API has **no native direct IP pairing endpoint**. `--via` does *not* mean direct peer-to-peer pairing; it is an alternative transport command (e.g., `wsl.exe -d Ubuntu -e sh -c` or a provider's authenticated CLI). Native direct pairing needs a separate authenticated Core protocol and upstream support. Do **not** replace it with unauthenticated TCP or an SSH command hidden behind a “Pair” button.

## UI behavior

1. List only local Core and registered remote machines, **never their sessions**.
2. Display separate connection states: connected, connecting, needs_login, needs_update, unreachable.
3. Show a **Retry** action for a disconnected machine, using `machine.connect`, not a new `machine.add` or `terminal.run`.
4. Refresh the machine list every **10 seconds while the modal is open**. Only a changed profile or health state notifies Workspace so the sidebar can update.
5. A failed background status refresh never discards the last successful list.
6. Display warnings when Herdr reports `keeps_running_after_logout == false`.
7. Do not echo custom `via` command arguments in the machine list; they can contain credentials.
8. Removing a machine requires confirmation and leaves its remote terminals running. The same applies while disconnected.

## Acceptance checks

On Windows, after pulling the branch:

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
cargo run --locked -- .
```

1. Open **Manage machines**; verify no remote session appears inside this modal.
2. Add a valid non-SSH transport using `via` (e.g., WSL) if available, or an SSH host already configured for Herdr.
3. Stop network access or remote Core temporarily: status should transition to disconnected/unreachable without destroying the agent viewers. Restore connectivity: Herdr retries; **Retry** also requests a redial.
4. Exercise `needs_login` and `needs_update` if your environment can produce them, and verify the guidance.
5. Remove an offline machine and confirm that the Core forgets its profile; remote processes remain intact.
6. Leave Manage machines open and confirm the status refreshes automatically, then close it and ensure it no longer polls.

Do not merge this stage into `main` until local Rust tests, clippy and manual Herdr tests pass.

## Next prerequisite for truly native pairing

A future Herdr Core change would need an explicit authenticated pairing protocol (trusted peer identity, authorization/consent, credential storage and rotation, transport encryption, discovery/endpoint selection, revocation, and compatible versions). No such API is assumed by this stage.
