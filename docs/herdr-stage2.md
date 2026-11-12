# Herdr-backed GPUI terminals — stage 2

The application can now use **Herdr Core** to own PTY processes while
**Alacritty Terminal + GPUI** continue to parse and paint their output.
This is opt-in so an existing Gentle Shell installation behaves exactly
as before unless the environment variable is set.

## Requirements

- Herdr Core (hc) installed on the machine running the GPUI app.
- A working Rust toolchain to build the fork.
- For Linux, the GPUI/GTK/WebKit dependencies already required by the project.

Herdr Core is the runtime; the GUI does not need an external
Alacritty terminal executable. hc connect starts or finds the local core.
Neither Pi RPC agent panels nor the original terminal code is replaced.

## Launch: PowerShell (Windows)

    hc --version
    $env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
    cargo run --locked -p gentle-shell-gpui

## Launch: bash/zsh (Linux/macOS)

    hc --version
    GENTLE_SHELL_TERMINAL_BACKEND=herdr cargo run --locked -p gentle-shell-gpui

Open a terminal tab/pane as usual (the command palette or New Terminal).
The title initially shows the shell location, then hc:tN (or the shell's
OSC title). Type commands normally. Confirm that Herdr owns the shell
with hc ls in another terminal. Resizing the pane should resize the
remote PTY. When you close the pane, Herdr keeps the process running
and the GUI merely detaches from its viewer channel.

Do not use hc kill to close the GUI viewer: it kills the managed
terminal, unlike the viewer CLOSE frame.

To test attaching to an existing session, set
GENTLE_SHELL_HERDR_TERMINAL=tN before starting the app (where tN
is a real address from hc ls); opening any GPUI terminal pane
then attaches to that address rather than creating a new shell.
This is a diagnostic override, not the automatic tab-restoration UX
planned for stage 3. Do not open multiple panes with the override set
unless you intentionally want multiple viewers of the same terminal.

If hc is in a nonstandard location, set
GENTLE_SHELL_HERDR_HC to its executable path.

To revert to the original terminal backend, unset the variable:

    # PowerShell
    Remove-Item Env:\GENTLE_SHELL_TERMINAL_BACKEND

    # bash/zsh
    unset GENTLE_SHELL_TERMINAL_BACKEND

Then restart the app.

## Architecture

- crates/herdr-client/src/terminal.rs: starts a shell through the
  plain JSON-RPC terminal.run API (or uses an existing address),
  then negotiates framed mode on a fresh hc connect bridge.
- terminal.attach creates the viewer channel; the stream reader
  forwards DATA to the UI. The writer sends keyboard data,
  RESIZE, FOCUS, CREDIT and CLOSE frames. Terminal-generated
  query replies use AUTOMATIC_INPUT when both ends support it.
- crates/app/src/features/terminal/mod.rs: retains the existing GPUI
  canvas and Alacritty Term, uses a persistent ANSI Processor to
  consume Herdr's DATA chunks, sends CREDIT only after processing,
  and translates GUI keyboard, focus and resize interactions.
- TerminalSession::drop sends CLOSE: no terminal.kill, no hc stop,
  and no exit of the core daemon.

## Validation

    cargo test --locked -p herdr-client
    cargo clippy --locked -p herdr-client --all-targets -- -D warnings
    cargo test --workspace --locked

The herdr-client unit tests cover frame layout, little-endian fields,
size guards, strict JSON-RPC response IDs, and the original stage-1
plain-mode handshake. Real interaction with Herdr still requires the
manual smoke test described above.

## Known limits (stage 3 and follow-up)

- Only terminal panes use Herdr; graphical agent tabs continue Pi RPC.
- The backend is selected per process through an environment variable,
  not yet through settings UI or per-pane controls.
- Existing sessions are not discovered/reconnected automatically after
  app restart; the address override is a manual workaround.
- Connection failure is displayed in the pane. Automatic reconnection,
  reconnect backoff, health checks and startup timeouts are still pending.
- Windows ConPTY/Win32-input-mode, advanced keyboard protocols,
  mouse-reporting compatibility and unusual full-screen TUI behavior
  need real cross-platform smoke testing. Not all are guaranteed yet.
- Future stage 3 should preserve terminal addresses in workspace state,
  add a selector for sessions/machines and handle connection loss.

Herdr protocol documentation:
https://github.com/herdrdev/herdr-core/blob/main/schema/README.md
