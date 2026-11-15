# Remote Files loading diagnostics (stage 7)

The desktop can measure a remote Files directory load without changing the
remote filesystem or starting a second Pi agent. Tracing is **opt-in**.

## Windows PowerShell 7

Run these from the desktop repository checkout:

```powershell
git switch feature/herdr-stage7-unified-desktop
git pull --ff-only origin feature/herdr-stage7-unified-desktop

node --check crates/pi-rpc/src/herdr_remote_bridge.cjs
cargo test --workspace --locked

$env:GENTLE_SHELL_TERMINAL_BACKEND = "herdr"
$env:GENTLE_SHELL_HERDR_FS_TRACE = "1"
cargo run --locked -- . 2>&1 | Tee-Object -FilePath .\herdr-files-trace.log
```

In the application, open **Machines**, choose the remote machine, and start a
**new remote agent** in the directory that loads slowly (for example
`/srv/workspaces/JaviPi`). Select its chat pane and open **Remote Files**.
Try **Refresh** and a subdirectory as well.

A new remote agent is necessary for the injected Node bridge to contain the
new server-side diagnostics. Existing agents are left running and may return
`remote_timings=unavailable`.

## What the log means

Look for lines prefixed with `[herdr-fs-trace]`:

- `UI view created ... rpc_ready=false`: Files was opened before a Pi RPC
  process existed. The UI retry loop attempts reconnecting to the **same**
  agent; it does not create another process.
- `UI load waiting ... rpc_ready=false`: the first list did not start yet.
- `RPC start` without a corresponding `RPC done`/`RPC error`: the
  request is still in flight, so the bottleneck may be transport or an
  operation inside the remote bridge.
- `client_ms`: total elapsed on the desktop while awaiting the RPC response.
- `remote_ms`: time spent inside the remote Node bridge before encoding
  its response. Their difference includes Herdr transport, scheduling,
  serialization and client-side delivery.
- `resolve_ms`: validating and resolving the directory with Linux realpath.
- `readdir_ms`: reading directory entries from the filesystem.
- `classify_ms`: filtering and sorting directory entry metadata.
- `gitignore_ms`: checking ignored paths with Git in sequential batches.
  The current bridge checks groups of 100 and each Git process can take up to
  eight seconds. A large value here can explain a slow Files panel.
- `filter_ms`: preparing the final visible list.
- `scanned` / `returned`: entries scanned and entries returned to GPUI.
  The return count is capped at 500, but the initial listing scans the entire
  directory.
- `git_batches` / `git_errors`: number of Git ignore subprocesses and
  failures; `git_errors` may include directories outside Git repositories.
- `UI load finished ... elapsed_ms`: time from the UI request until the
  updated file tree has been built.

A profile is added to `herdr_fs_list` responses **only when**
`GENTLE_SHELL_HERDR_FS_TRACE=1` on the desktop, and has timing/count
numbers only. In normal mode there is no server-side profiling payload or
additional Git query. Logs can include local and Linux directory paths,
so review them before sharing.

## Disable tracing

Close the app, then in PowerShell:

```powershell
Remove-Item Env:GENTLE_SHELL_HERDR_FS_TRACE -ErrorAction SilentlyContinue
```

To validate performance, compare at least two runs in the **same** directory,
one immediately after starting the agent and another after clicking Refresh.
