# Herdr Core integration — stage 1

This fork now contains the independent Rust crate at
crates/herdr-client. It does not alter the GPUI terminal view or launch any
agent. The original Alacritty PTY remains the default in the desktop app.

## What works

- Launches the installed hc connect bridge on Windows, Linux or macOS.
- Connects using newline-delimited JSON-RPC 2.0 (plain mode).
- Performs the hello handshake and validates protocol generation 1.
- Checks that terminal.list is advertised, and lists existing terminals.
- Provides a reusable plain-mode request method with remote error details.
- Tears down the bridge process when the client is dropped, not the core or
  the terminal processes it manages.
- Tests request serialization, response IDs, errors, truncated responses,
  handshake validation and terminal discovery parsing.

Requires hc on PATH (or a supplied executable path). The bridge starts the
core as needed, according to hc's own behavior.

## Smoke test

With Rust and herdr-core installed, in the repo root:

    cargo test --locked -p herdr-client
    cargo run --locked -p herdr-client --example probe

To supply the path to hc explicitly:

    cargo run --locked -p herdr-client --example probe -- /path/to/hc

Expected output includes the core name, version, protocol generation, and
the number of visible terminals. An empty terminal list is a successful
connection as long as the handshake succeeded.

## Scope and limitations

This is the connection/discovery foundation, not the terminal integration.
It does **not** replace Alacritty's existing PTY loop, keep any GPUI tabs
alive, forward input/output, subscribe to status events, or attach to a
terminal. Those require framed mode (hello with framed:true), a binary
multiplexer, window sizing, credit-based flow control, and reconnection.

The public client is synchronous and can block if hc or the server stalls;
when used by GPUI it must run on a worker thread, not the UI thread.
Production integration should add bounded timeouts, reconnection,
and framed-mode support before exposing a terminal backend switch.

Reference:
https://github.com/herdrdev/herdr-core/blob/main/schema/README.md
