# Stage 4A — Local Pi RPC over Herdr Core (draft)

## Goal

In HERDR mode, not just terminal panes but every Gentle Shell agent tab
starts through the local Herdr Core. LOCAL mode keeps the existing native
PiProcess transport and never calls Herdr. There is no automatic fallback.

4A uses Herdr on the same computer as GPUI. Choosing a remote host or running
the GPUI bridge binary on a different OS is explicitly out of scope until 4B.

## Implementation

- A new pi-rpc RpcProcess wrapper preserves the existing request/response and
  Incoming event interfaces in the application. The app selects exactly one
  backend according to GENTLE_SHELL_TERMINAL_BACKEND.
- HerdrAgentProcess launches the current GPUI executable as a Herdr-managed
  terminal with argv [exe, --herdr-agent-bridge, token, ...].
- The bridge launches gentle-shell --mode rpc in a subprocess with real stdin/
  stdout pipes, preserving interactive Pi JSONL and extension UI requests.
- The Herdr PTY is not a transparent byte stream; JSONL is therefore chunked
  into base64 frames (maximum 96 unencoded bytes per frame). Only tagged,
  token-matching OUT frames become Pi RPC data. Echoed input and arbitrary
  terminal noise are discarded.
- Startup waits for an explicit READY frame. Request IDs are correlated to
  responses. The GUI still receives the same Incoming event types as native.
- Transport loss fails the agent tab closed: no silent fallback to a native
  process and no replay of prompts after connection loss.
- On shutdown or failed startup, only the bridge terminal owned by the agent
  is killed through terminal.kill. Herdr Core itself stays running.
- Existing session --session paths are forwarded to the bridge on startup.

Herdr Core's machine.exec is not used: it is a bounded one-shot execution
interface, unsuitable for an indefinitely running bidirectional Pi agent.

## Windows checkout

    git fetch https://github.com/javijec/gentle-shell-gpui-herdr-core.git feature/herdr-stage4a-agent-rpc
    git switch -c feature/herdr-stage4a-agent-rpc FETCH_HEAD
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked

If you already have this branch locally, fetch and fast-forward instead.

## Manual validation (42–48)

42. Launch GPUI with GENTLE_SHELL_TERMINAL_BACKEND=herdr.
    In a new agent tab, send a short prompt and observe the reply. An hc ls
    entry labeled gentle-shell=rpc-agent should appear for the agent.
43. Continue a conversation with several streaming responses; verify markdown,
    tool execution and message updates, not just a one-line get_state reply.
44. Test extensions' UI dialogs and commands: requests must flow both ways.
45. Open a second agent tab. Requests must go to their own process/ID channel
    without crossing messages.
46. Close an agent and verify only its owned Herdr terminal ends (the Herdr
    daemon and other terminals must remain alive).
47. Close GPUI and start it with GENTLE_SHELL_TERMINAL_BACKEND=local. Agent tabs
    must use native PiProcess. Machines must remain hidden; no hc process should
    start as a result of agent UI operations.
48. Start HERDR mode with GENTLE_SHELL_HERDR_HC pointing to a nonexistent
    executable. The agent must show a failure and must not open native Pi
    behind the scenes.

PowerShell:

    $env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
    cargo run --locked -p gentle-shell-gpui

For fallback verification, stop GPUI and run:

    $env:GENTLE_SHELL_TERMINAL_BACKEND = "local"
    cargo run --locked -p gentle-shell-gpui

## Known limits / not yet verified

- This branch has not been compiled or manually run on Windows by the author.
  It must remain a draft PR until Windows tests and clippy pass.
- Echo, wrapping, and Windows ConPTY behavior in a real Herdr PTY need a smoke
  test. The framed codec has synthetic round-trip and correlation tests, but
  those do not replace a real Pi RPC agent run.
- RPC sessions do not yet recover across a disconnected Herdr viewer (4D).
- Remote agent machine selection and remote bridge deployment are 4B/4C.
- The bridge is launched using the local GPUI executable, so this stage does
  not support starting it directly on JaviPi.
