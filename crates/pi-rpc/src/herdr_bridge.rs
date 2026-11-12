//! Shared Herdr Pi JSONL bridge for local GPUI and remote ARM64 agents.
//!
//! No GPUI dependency: the same framed RPC implementation runs on Windows,
//! Linux x86_64, and Linux aarch64. Child Pi uses ordinary byte-transparent
//! stdin/stdout pipes; Herdr only transports bounded base64 frames in a PTY.

use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde_json::{Value, json};

use crate::herdr_wire::{Decoder, encode, valid_token};

type Output = Arc<Mutex<io::Stdout>>;
const PROBE_ID: &str = "gs4a-startup";
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// The READY handshake is not satisfied until Pi itself answers a request.
fn probe_response(line: &[u8]) -> Option<Result<(), String>> {
    let value: Value = serde_json::from_slice(line).ok()?;
    if value["type"] != "response" || value["id"] != PROBE_ID {
        return None;
    }
    if value["success"] == true {
        Some(Ok(()))
    } else {
        Some(Err(value["error"].as_str().unwrap_or("get_state failed").to_owned()))
    }
}

fn emit(out: &Output, token: &str, kind: char, payload: &[u8]) -> io::Result<()> {
    let mut stream = out.lock().unwrap_or_else(|e| e.into_inner());
    for frame in encode(token, kind, payload) {
        stream.write_all(&frame)?;
    }
    stream.flush()
}

fn stream_lines(
    input: impl Read,
    out: Output,
    token: String,
    kind: char,
    mut ready: Option<mpsc::SyncSender<Result<(), String>>>,
    diagnostics: Option<Arc<Mutex<String>>>,
) -> io::Result<()> {
    let mut reader = io::BufReader::new(input);
    loop {
        let mut line = Vec::new();
        // A malformed child must not allocate unlimited memory in the bridge.
        let n = (&mut reader).take(8 * 1024 * 1024 + 1).read_until(b'\n', &mut line)?;
        if n == 0 {
            return Ok(());
        }
        if n > 8 * 1024 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "oversized Pi record"));
        }
        if kind == 'O'
            && let Some(response) = probe_response(&line)
        {
            if let Some(tx) = ready.take() {
                let _ = tx.try_send(response);
            }
            continue;
        }
        if kind == 'E'
            && let Some(diagnostics) = &diagnostics
        {
            let mut buffer = diagnostics.lock().unwrap_or_else(|e| e.into_inner());
            if buffer.len() < 4096 {
                buffer.push_str(&String::from_utf8_lossy(&line).chars().take(2048).collect::<String>());
            }
        }
        emit(&out, &token, kind, &line)?;
    }
}

/// Invoked only with --herdr-agent-bridge TOKEN [--session FILE].
/// It is not an interactive shell and must never write unframed stdout.
pub fn run(args: Vec<String>) -> i32 {
    match serve(&args) {
        Ok(code) => code,
        Err(error) => {
            if let Some(token) = args.first().filter(|token| valid_token(token)) {
                let output = Arc::new(Mutex::new(io::stdout()));
                let _ = emit(&output, token, 'E', format!("{error:#}").as_bytes());
                let _ = emit(&output, token, 'X', b"1");
            }
            eprintln!("Herdr RPC bridge: {error:#}");
            1
        }
    }
}

fn serve(args: &[String]) -> anyhow::Result<i32> {
    let token = args.first().ok_or_else(|| anyhow::anyhow!("missing bridge token"))?;
    anyhow::ensure!(valid_token(token), "invalid bridge token");
    anyhow::ensure!(
        args.len() == 1 || (args.len() == 3 && args[1] == "--session"),
        "unexpected bridge arguments"
    );
    let session = if args.len() == 3 {
        Some(PathBuf::from(&args[2]))
    } else {
        None
    };

    // The caller sets its own Pi configuration before entering the bridge.
    // A remote ARM64 bridge uses the remote machine's home and PATH, never
    // the Windows GPUI host's settings.
    let cwd = std::env::current_dir()?;
    let mut spec = crate::resolve_backend(&cwd)?;
    if let Some(session) = session {
        spec.args.push("--session".into());
        spec.args.push(session.to_string_lossy().into_owned());
    }

    let mut child = crate::command(&spec.program);
    child
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in &spec.env {
        child.env(name, value);
    }
    let mut child = child.spawn()?;
    let mut child_stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no Pi stdin"))?;
    let child_stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no Pi stdout"))?;
    let child_stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("no Pi stderr"))?;

    let output: Output = Arc::new(Mutex::new(io::stdout()));
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let diagnostics = Arc::new(Mutex::new(String::new()));
    let out_stdout = output.clone();
    let tok_stdout = token.clone();
    let stdout_thread = std::thread::spawn(move || {
        stream_lines(child_stdout, out_stdout, tok_stdout, 'O', Some(ready_tx), None)
    });
    let out_stderr = output.clone();
    let tok_stderr = token.clone();
    let log = diagnostics.clone();
    let stderr_thread = std::thread::spawn(move || {
        stream_lines(child_stderr, out_stderr, tok_stderr, 'E', None, Some(log))
    });

    // A process handle is not a successful Pi launch. Check actual JSONL RPC
    // before notifying GPUI: extensions, auth and configuration can fail.
    serde_json::to_writer(&mut child_stdin, &json!({
        "type": "get_state",
        "id": PROBE_ID
    }))?;
    child_stdin.write_all(b"\n")?;
    child_stdin.flush()?;

    let start = match ready_rx.recv_timeout(PROBE_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(anyhow::anyhow!("Pi get_state rejected startup: {error}")),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(anyhow::anyhow!("Pi exited before responding to get_state"))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(anyhow::anyhow!("Pi did not respond to get_state within 15 seconds"))
        }
    };
    if let Err(error) = start {
        let _ = child.kill();
        let _ = child.wait();
        let _ = stdout_thread.join();
        let _ = stderr_thread.join();
        let stderr = diagnostics.lock().unwrap_or_else(|e| e.into_inner());
        anyhow::bail!("Gentle Shell agent startup failed: {error}. {stderr}");
    }

    // Only now is the agent actually ready for GPUI's Pi RPC requests.
    emit(&output, token, 'R', b"")?;

    let input_token = token.clone();
    let input_log = output.clone();
    let _input_thread = std::thread::spawn(move || {
        let mut decoder = Decoder::new(input_token.clone());
        let mut stdin = io::stdin();
        let mut chunk = [0_u8; 2048];
        loop {
            let count = match stdin.read(&mut chunk) {
                Ok(0) => {
                    let _ = emit(&input_log, &input_token, 'E', b"Herdr bridge PTY stdin reached EOF");
                    break;
                }
                Ok(count) => count,
                Err(error) => {
                    let _ = emit(
                        &input_log, &input_token, 'E',
                        format!("Herdr bridge PTY input error: {error}").as_bytes(),
                    );
                    break;
                }
            };
            for frame in decoder.push(&chunk[..count]) {
                if frame.kind == 'I'
                    && let Err(error) = child_stdin.write_all(&frame.payload)
                {
                    let _ = emit(
                        &input_log, &input_token, 'E',
                        format!("Pi RPC stdin write failed: {error}").as_bytes(),
                    );
                    return;
                }
            }
            if let Err(error) = child_stdin.flush() {
                let _ = emit(
                    &input_log, &input_token, 'E',
                    format!("Pi RPC stdin flush failed: {error}").as_bytes(),
                );
                return;
            }
        }
    });

    let status = child.wait()?;
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    let code = status.code().unwrap_or(1);
    emit(&output, token, 'X', code.to_string().as_bytes())?;
    Ok(code)
}

#[cfg(test)]
mod startup_probe_tests {
    use super::probe_response;

    #[test]
    fn ready_requires_successful_pi_get_state_response() {
        assert_eq!(probe_response(
            br#"{"type":"response","id":"gs4a-startup","success":true,"data":{}}"#
        ), Some(Ok(())));
        assert_eq!(probe_response(
            br#"{"type":"response","id":"another-id","success":true}"#
        ), None);
        assert_eq!(probe_response(br#"{"type":"agent_start"}"#), None);
    }

    #[test]
    fn failed_probe_is_not_reported_as_ready() {
        assert_eq!(probe_response(
            br#"{"type":"response","id":"gs4a-startup","success":false,"error":"extension failed"}"#
        ), Some(Err("extension failed".to_owned())));
    }
}
