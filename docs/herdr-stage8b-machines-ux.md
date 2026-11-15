# Stage 8B — Machines sidebar UX

The Stage 8A machine management and reconnection API is stable in `main`. This branch changes **presentation and navigation only**, and adds an optional direct retry action via the same existing `machine.connect` method.

## Expected behavior

- The left Machines panel is always present: either the full tree or the narrow **48px rail**. `Ctrl+B` and the toolbar button switch these modes.
- The compact rail is a **direct agent/terminal switcher**. Each machine is a short header with its Herdr connectivity dot; **below it are the individually clickable agent and terminal icons** belonging to that machine. Session tooltips include their title, runtime folder, address where known, and owning machine.
- **Single click on a session icon switches directly to that agent/terminal while Machines stays collapsed at 48 px.** This uses the same selection path as the expanded navigator: focus an already open pane (even in another workspace), restore a saved workspace session, or reattach to an existing live Herdr terminal. No new agent is created as a side effect of navigation.
- The active session icon is highlighted in the rail; a running session has a status dot. This does not rely on the full machine group being expanded.
- Clicking **the machine header**, rather than a session icon, expands that machine's full sidebar group. Machine headers also show the authoritative connectivity state; saved sessions alone do not imply an online machine.
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

1. Open **two different agents** (for example Windows and JaviPi) and one terminal. Focus each and verify the correct runtime title.
2. Collapse with `Ctrl+B`. Verify the 48px rail shows a machine header **and separate clickable icons for every agent/terminal**. The session tooltip must identify its title and machine.
3. Click agent A and agent B **without expanding Machines**. The visible agent must switch, the correct icon must highlight, and the rail must remain exactly 48 px wide.
4. Click a session in a *different workspace* from the rail and verify focus changes to it. If a saved or not-yet-attached live Herdr agent is shown, clicking must recover/attach that session rather than create a duplicate.
5. Click a machine **header**, not its session icon. Only then should the full navigator expand with that group's rows visible.
6. Expand and collapse again: the previous full sidebar width should be restored.
7. Temporarily disconnect JaviPi. Check the offline indicator and status tooltip. Use the machine's `...` menu to retry, then restore connectivity. The agent must not be duplicated or terminated.
8. Open Manage machines from the `...` menu. It must show registered machines, **not session rows**.
9. Switch to local backend: local navigation remains available. Re-enable Herdr: remote machine and session ownership should be preserved.

**Limitations for follow-up:** drag-and-drop custom machine ordering and pixel-perfect parity with the upstream Herdr UI are not included here; they require explicit persistent ordering semantics and visual validation. No direct-IP pairing is invented or implied.
