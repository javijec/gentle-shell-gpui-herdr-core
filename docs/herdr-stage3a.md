# Herdr Core integration — stage 3A: persistent session identities

This first stage-3 increment restores existing Herdr terminals when a
saved workspace is reopened. It builds on stage 2 and is intentionally
conservative: it does not create a replacement process if the original
terminal is unavailable.

## What is implemented

- Every Herdr-backed terminal pane saves its herdr_terminal address to
  workspaces.json, alongside the current cwd and pane layout.
  Examples: t7, javipi/t7.
- New terminals report the permanent address immediately after
  terminal.run succeeds; the workspace periodic title/state polling
  triggers a debounced save.
- Restored panes use the stored address with terminal.attach. If it
  fails, they never run a duplicate terminal.
- When a stored Herdr address is present, GPUI selects Herdr for that pane
  even if the backend environment variable is not set. Set
  GENTLE_SHELL_TERMINAL_BACKEND=local explicitly to choose local PTYs.
- The initial attachment makes up to 5 attempts with backoffs of
  1/2/4/8 seconds. It displays each failure in the panel; success
  clears the warning. Detaching a pane during a retry cancels the retries.
- The discovery client gains machine.list, and its probe example prints
  registered machine names.
- SavedPane's new optional field defaults to None, preserving compatibility
  with workspace files created by older builds.

## Windows PowerShell: build and check

    git fetch origin
    git switch --track origin/feature/herdr-stage3-session-restore
    cargo test --locked -p herdr-client
    cargo test --locked -p gentle-shell-gpui
    cargo clippy --workspace --all-targets --locked -- -D warnings

## Windows PowerShell: smoke test

Start the GUI with Herdr for new terminals:

    hc --version
    $env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
    cargo run --locked -p gentle-shell-gpui

Open a terminal pane and type a command such as Get-Location.
In another shell:

    hc ls

Note its address (tN). Close the whole application (not the terminal
tab); the managed terminal should remain in hc ls. Reopen the GUI.
The workspace should show the same hc:tN terminal and its current screen,
without creating a second terminal. If you closed the pane/tab, that pane
was intentionally removed from the saved workspace; its Herdr process still
exists, but it will not be reopened automatically.

You can also inspect the app's workspaces.json in the OS data directory.
Each Herdr terminal pane should include herdr_terminal with its address.
Make a backup before editing this JSON manually.

## Remote machine smoke test

Register your remote machine using Herdr's CLI:

    hc machine add dietpi@javipi --name javipi
    hc machine ls

Start a terminal on that registered machine:

    hc run --on javipi --bg

To temporarily attach a GPUI pane to that remote terminal, use the
diagnostic override while launching the GUI:

    $env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
    $env:GENTLE_SHELL_HERDR_TERMINAL = "javipi/t7"
    cargo run --locked -p gentle-shell-gpui

Replace javipi/t7 with the actual address returned by hc ls.
Open one terminal pane and verify it is attached to the remote terminal.
Close the app, clear the diagnostic override, and reopen it. The saved
workspace should remember the remote address without needing that
environment variable again. Avoid opening multiple new tabs with the
override set: they would deliberately share the same terminal.

Remove the diagnostic override in PowerShell:

    Remove-Item Env:\GENTLE_SHELL_HERDR_TERMINAL

To force local PTYs after a previous Herdr session was saved:

    $env:GENTLE_SHELL_TERMINAL_BACKEND = "local"

This discards the stored Herdr address for that pane the next time the
workspace is saved; the process on Herdr itself remains running.

## Out of scope / remaining work

- Not yet continuous live reconnection. If an attached stream dies
  after it has been established, the pane currently displays an error.
  Retry logic covers only the initial connection to a persisted session.
- Full settings UI, terminal picker and machine picker remain pending.
  Remote terminals work by a registered machine address and manual
  diagnostic override, not by clicking on a machine list yet.
- Advanced ConPTY/keyboard/mouse behavior still requires manual testing.
- No migration/association of anonymous processes created before this
  version. Only panes saved with an address can restore automatically.
- New terminal creation and session recovery must be smoke-tested on
  Windows, Linux and macOS with a real hc install before merging.

Protocol reference: https://github.com/herdrdev/herdr-core/blob/main/schema/README.md
