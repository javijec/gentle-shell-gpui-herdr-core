# Stage 8B — Machines sidebar UX

The Stage 8A machine management and reconnection API is stable in `main`. This branch changes **presentation and navigation only**, and adds an optional direct retry action via the same existing `machine.connect` method.

## Expected behavior

- The left Machines panel is always present: either the full tree or the narrow **48px rail**. `Ctrl+B` and the toolbar button switch these modes.
- The rail distinguishes the machine that owns the currently focused agent/terminal, displays the authoritative Herdr connectivity state with a small dot and shows the count of its sessions.
- Clicking a machine in the rail expands **that machine's** group in the full sidebar, even if that group had previously been collapsed. Clicking a machine does **not** switch the currently running agent, start a terminal, or kill a remote process.
- The expanded sidebar highlights the same machine as the rail. Its status tooltip states connected, connecting, needs_login, needs_update, unreachable, or unknown. Saved sessions alone must never imply a live remote machine.
- A machine's `...` menu provides **Retry connection** (if offline) and **Manage machines**. Retry is an asynchronous `machine.connect` RPC to Herdr Core; it never performs `terminal.run` or `machine.add`.
- Local and remote agents/terminals continue to use their original machine ownership and workspace persistence. Narrowing/expanding the panel retains its saved expanded width.

## Regression checklist

```powershell
git fetch origin
git switch feature/herdr-stage8b-machines-ux
git pull --ff-only origin feature/herdr-stage8b-machines-ux
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
cargo run --locked -- .
```

1. Open one local agent and one remote JaviPi agent. Focus each and confirm the highlighted machine and title reflect the same runtime.
2. Collapse with `Ctrl+B`. Verify the 48px rail shows each machine's icon, status and session count (including zero).
3. Expand a **collapsed** machine group from its rail icon. The corresponding group should open; the current agent must remain unchanged.
4. Expand and collapse again: the previous full sidebar width should be restored.
5. Temporarily disconnect JaviPi. Check the offline indicator and status tooltip. Use the machine's `...` menu to retry, then restore connectivity. The agent must not be duplicated or terminated.
6. Open Manage machines from the `...` menu. It must show registered machines, **not session rows**.
7. Switch to local backend: local navigation remains available. Re-enable Herdr: remote machine and session ownership should be preserved.

**Limitations for follow-up:** drag-and-drop custom machine ordering and pixel-perfect parity with the upstream Herdr UI are not included here; they require explicit persistent ordering semantics and visual validation. No direct-IP pairing is invented or implied.
