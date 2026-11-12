//! Stage 4A: run Pi JSONL behind a Herdr-managed PTY on this computer.
//!
//! This subcommand never enters GPUI or single-instance handling. It turns
//! the PTY into two bounded, base64-encoded data channels so terminal echo,
//! CR/LF conversion, and escape sequences cannot be mistaken for Pi records.

use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use pi_rpc::herdr_wire::{Decoder, encode, valid_token};

type Output = Arc<Mutex<io::Stdout>>;

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
        emit(&out, &token, kind, &line)?;
    }
}

/// Invoked only with --herdr-agent-bridge TOKEN [--session FILE].
/// It is not an interactive shell and must never write unframed stdout.
pub fn run(args: Vec<String>) -> i32 {
    match serve(&args) {
        Ok(code) => code,
        Err(error) => {
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

    // Resolve the same bundled/runtime Gentle Shell as a local agent.
    // This bridge itself is managed by Herdr; its Pi child uses real pipes.
    crate::features::setup::apply_toolchain_path();
    pi_rpc::set_system_prompt(crate::features::devices::agent_instructions(
        crate::features::setup::devices::prefs(),
    ));
    let cwd = std::env::current_dir()?;
    let mut spec = pi_rpc::resolve_backend(&cwd)?;
    if let Some(session) = session {
        spec.args.push("--session".into());
        spec.args.push(session.to_string_lossy().into_owned());
    }

    let mut child = pi_rpc::command(&spec.program);
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
    let out_stdout = output.clone();
    let tok_stdout = token.clone();
    let stdout_thread = std::thread::spawn(move || {
        stream_lines(child_stdout, out_stdout, tok_stdout, 'O')
    });
    let out_stderr = output.clone();
    let tok_stderr = token.clone();
    let stderr_thread = std::thread::spawn(move || {
        stream_lines(child_stderr, out_stderr, tok_stderr, 'E')
    });

    // The agent is now accepting Pi JSONL through the bridge.
    emit(&output, token, 'R', b"")?;

    let input_token = token.clone();
    std::thread::spawn(move || {
        let mut decoder = Decoder::new(input_token);
        let mut stdin = io::stdin();
        let mut chunk = [0_u8; 2048];
        while let Ok(count) = stdin.read(&mut chunk) {
            if count == 0 {
                break;
            }
            for frame in decoder.push(&chunk[..count]) {
                if frame.kind == 'I' && child_stdin.write_all(&frame.payload).is_err() {
                    return;
                }
            }
            if child_stdin.flush().is_err() {
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
