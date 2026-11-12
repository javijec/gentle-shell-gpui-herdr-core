//! Framed Herdr terminal stream. Deliberately independent of GPUI.
//!
//! A single worker owns the hc connect process and the output reader; a
//! separate writer thread serializes input, resize, focus and flow-control
//! frames. Closing a viewer detaches; it never kills the managed program.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{Error, HerdrClient, PROTOCOL_GENERATION, RemoteError, Result};

const MAX_FRAME: usize = 16 * 1024 * 1024;
const DATA: u8 = 0;
const CREDIT: u8 = 1;
const CLOSE: u8 = 2;
const RESIZE: u8 = 3;
const FOCUS: u8 = 4;
const AUTOMATIC_INPUT: u8 = 6;

/// Events emitted by the background I/O worker. Consume outside the UI
/// thread, or forward them to GPUI's async executor.
#[derive(Debug)]
pub enum TerminalEvent {
    /// A permanent address has been allocated or selected; persist immediately.
    Assigned(String),
    /// Transport failed; a background reconnect attempt is scheduled.
    Retrying(String),
    /// A new framed viewer channel is ready. Recovered signals that the ANSI
    /// parser and painted screen must reset before the first replacement paint.
    Attached { address: String, recovered: bool },
    Output { generation: u64, bytes: Vec<u8> },
    /// Retries are exhausted; the viewer remains open for a manual retry.
    NeedsReconnect(String),
    Closed,
    Failed(String),
}

enum Command {
    Input(Vec<u8>, bool),
    Credit(u64, u32),
    Resize(u16, u16),
    Focus(bool),
    Reconnect,
    Detach,
    /// Internal signal to release the writer thread after its reader ends.
    StopWriter,
}

/// Latest GUI state; kept across channel changes and connection outages.
#[derive(Clone, Copy)]
struct DesiredState {
    cols: u16,
    rows: u16,
    focused: bool,
}

/// Shared I/O state for a terminal worker and each successive viewer channel.
/// Kept as one unit so reconnect paths use the same cancel flag, target size
/// and event reporter without passing many unrelated arguments.
struct ConnectionState<'a> {
    tx: &'a Sender<Command>,
    cancelled: &'a AtomicBool,
    connected: &'a AtomicBool,
    desired: &'a Mutex<DesiredState>,
    report: &'a dyn Fn(TerminalEvent),
}

/// One GPUI terminal pane, with stable identity across transport reconnects.
/// Closing it detaches the viewer, never kills the process owned by Herdr.
pub struct TerminalSession {
    tx: Sender<Command>,
    cancelled: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    desired: Arc<Mutex<DesiredState>>,
}

impl TerminalSession {
    /// Starts the session on a worker thread; this constructor does not
    /// block GPUI. Startup errors arrive through the callback.
    ///
    /// GENTLE_SHELL_HERDR_HC overrides the hc executable.
    /// GENTLE_SHELL_HERDR_TERMINAL attaches to an existing address.
    pub fn spawn(
        cwd: PathBuf,
        resume: Option<String>,
        cols: u16,
        rows: u16,
        report: impl Fn(TerminalEvent) + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicBool::new(false));
        let desired = Arc::new(Mutex::new(DesiredState {
            cols: cols.max(1),
            rows: rows.max(1),
            focused: false,
        }));
        let worker_tx = tx.clone();
        let worker_cancelled = cancelled.clone();
        let worker_connected = connected.clone();
        let worker_desired = desired.clone();
        std::thread::spawn(move || {
            let state = ConnectionState {
                tx: &worker_tx,
                cancelled: &worker_cancelled,
                connected: &worker_connected,
                desired: &worker_desired,
                report: &report,
            };
            if let Err(error) = serve(cwd, resume, rx, &state)
                && !worker_cancelled.load(Ordering::SeqCst)
            {
                report(TerminalEvent::Failed(error.to_string()));
            }
        });
        Self { tx, cancelled, connected, desired }
    }

    /// Human input while offline is intentionally discarded, not replayed.
    pub fn input(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() && self.connected.load(Ordering::SeqCst) {
            let _ = self.tx.send(Command::Input(bytes, false));
        }
    }

    /// Emulator-generated replies retain their automatic-input tag.
    pub fn automatic_input(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() && self.connected.load(Ordering::SeqCst) {
            let _ = self.tx.send(Command::Input(bytes, true));
        }
    }

    /// Credit belongs to one viewer generation. Old credit must never be
    /// applied to a new Herdr channel after reconnect.
    pub fn consumed(&self, generation: u64, bytes: usize) {
        if let Ok(amount) = u32::try_from(bytes) {
            let _ = self.tx.send(Command::Credit(generation, amount));
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        {
            let mut desired = self.desired.lock().unwrap_or_else(|e| e.into_inner());
            desired.cols = cols;
            desired.rows = rows;
        }
        if self.connected.load(Ordering::SeqCst) {
            let _ = self.tx.send(Command::Resize(cols, rows));
        }
    }

    pub fn focus(&self, focused: bool) {
        self.desired.lock().unwrap_or_else(|e| e.into_inner()).focused = focused;
        if self.connected.load(Ordering::SeqCst) {
            let _ = self.tx.send(Command::Focus(focused));
        }
    }

    /// Re-attempt the same terminal address after automatic retries ran out.
    pub fn reconnect(&self) {
        if !self.cancelled.load(Ordering::SeqCst) {
            let _ = self.tx.send(Command::Reconnect);
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Command::Detach);
    }
}

struct Frame {
    channel: u32,
    kind: u8,
    payload: Vec<u8>,
}

fn encode_frame(channel: u32, kind: u8, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > MAX_FRAME {
        return Err(Error::Protocol("framed payload exceeds 16 MiB".into()));
    }
    let mut out = Vec::with_capacity(9 + payload.len());
    out.extend_from_slice(&channel.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

fn write_frame(writer: &mut impl Write, channel: u32, kind: u8, payload: &[u8]) -> Result<()> {
    writer.write_all(&encode_frame(channel, kind, payload)?)?;
    writer.flush()?;
    Ok(())
}

fn read_frame(reader: &mut impl Read) -> Result<Frame> {
    let mut header = [0u8; 9];
    reader.read_exact(&mut header)?;
    let channel = u32::from_le_bytes(header[..4].try_into().unwrap());
    let kind = header[4];
    let length = u32::from_le_bytes(header[5..9].try_into().unwrap()) as usize;
    if length > MAX_FRAME {
        return Err(Error::Protocol("received oversized Herdr frame".into()));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    Ok(Frame { channel, kind, payload })
}

/// Closing this bridge stops only the hc connect forwarding process.
/// The herdr-core daemon and any attached terminal remain alive.
struct Bridge {
    // The writer can terminate hc connect on viewer close, unblocking
    // a reader that would otherwise stay inside read_exact indefinitely.
    child: Arc<Mutex<Child>>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Bridge {
    fn terminate(&self) {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Bridge {
    fn connect(executable: &OsString) -> Result<(Self, bool)> {
        let mut child = ProcessCommand::new(executable)
            .arg("connect")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Protocol("hc connect has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Protocol("hc connect has no stdout".into()))?;
        let mut bridge = Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
        };
        let hello = json!({
            "jsonrpc": "2.0", "id": 1, "method": "hello",
            "params": {
                "generation": PROTOCOL_GENERATION,
                "framed": true,
                "features": ["automatic_input"]
            }
        });
        let writer = bridge.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *writer, &hello)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        let mut line = Vec::new();
        let n = (&mut bridge.stdout)
            .take(1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 || n > 1024 * 1024 || !line.ends_with(b"\n") {
            return Err(Error::Protocol("missing or excessive Herdr hello reply".into()));
        }
        let reply: Value = serde_json::from_slice(&line)?;
        let result = check_response(reply, 1)?;
        if result["generation"].as_u64() != Some(PROTOCOL_GENERATION as u64)
            || result["framed"] != true
        {
            return Err(Error::Protocol("Herdr did not negotiate framed protocol generation 1".into()));
        }
        let automatic = result["features"]
            .as_array()
            .is_some_and(|f| f.iter().any(|x| x.as_str() == Some("automatic_input")));
        Ok((bridge, automatic))
    }

    fn attach(&mut self, address: &str, cols: u16, rows: u16, resumed: bool) -> Result<(u32, bool)> {
        let request = json!({
            "jsonrpc": "2.0", "id": 2, "method": "terminal.attach",
            "params": {
                "terminal": address, "cols": cols, "rows": rows,
                "observe": false, "history": 0, "resumed": resumed
            }
        });
        write_frame(self.stdin.as_mut().unwrap(), 0, DATA, request.to_string().as_bytes())?;
        // Herdr promises that the attach response precedes stream frames.
        let reply = read_frame(&mut self.stdout)?;
        if reply.channel != 0 || reply.kind != DATA {
            return Err(Error::Protocol("Herdr did not reply to terminal.attach on RPC channel".into()));
        }
        let response: Value = serde_json::from_slice(&reply.payload)?;
        let result = check_response(response, 2)?;
        let channel = result["channel"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|ch| *ch != 0)
            .ok_or_else(|| Error::Protocol("terminal.attach returned an invalid channel".into()))?;
        let automatic = result["automatic_input"].as_bool().unwrap_or(false);
        Ok((channel, automatic))
    }
}

fn check_response(reply: Value, id: u64) -> Result<Value> {
    if reply["jsonrpc"] != "2.0" || reply["id"].as_u64() != Some(id) {
        return Err(Error::Protocol(format!("invalid response for request {id}")));
    }
    match (reply.get("result"), reply.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) => Err(Error::Remote(
            serde_json::from_value::<RemoteError>(error.clone())?,
        )),
        _ => Err(Error::Protocol("missing or ambiguous JSON-RPC result".into())),
    }
}

/// A CLOSE carrying a final process reason is not a transient network loss.
/// A bare CLOSE ends only the viewer, and remote/lost/keep reasons can recover.
fn terminal_finished(payload: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(payload) else {
        return false;
    };
    matches!(
        value["reason"].as_str(),
        Some("exited" | "killed")
    )
}

fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs(1_u64 << (failures.saturating_sub(1).min(3)))
}

/// A control queue must not replay bytes or credit from an old viewer once
/// the network comes back. Only the latest resize/focus in DesiredState
/// survives the outage.
fn discard_offline(rx: &Receiver<Command>, cancelled: &AtomicBool) -> bool {
    while let Ok(command) = rx.try_recv() {
        if matches!(command, Command::Detach) {
            cancelled.store(true, Ordering::SeqCst);
            return false;
        }
    }
    !cancelled.load(Ordering::SeqCst)
}

fn wait_retry(
    rx: &Receiver<Command>,
    cancelled: &AtomicBool,
    delay: Option<Duration>,
) -> bool {
    // None = automatic retries exhausted; wait until the user explicitly
    // clicks Reconnect (or closes the pane).
    match delay {
        None => loop {
            if cancelled.load(Ordering::SeqCst) { return false; }
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(Command::Reconnect) => return true,
                Ok(Command::Detach) | Err(RecvTimeoutError::Disconnected) => return false,
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            }
        },
        Some(delay) => {
            let deadline = Instant::now() + delay;
            loop {
                if cancelled.load(Ordering::SeqCst) { return false; }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() { return true; }
                match rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
                    Ok(Command::Reconnect) => return true,
                    Ok(Command::Detach) | Err(RecvTimeoutError::Disconnected) => return false,
                    Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                }
            }
        }
    }
}

enum StreamEnd {
    Finished,
    Lost(String),
}

/// Returns the command receiver for reuse by the next connection. The
/// writer owns it while the reader blocks on hc connect stdout.
fn stream(
    mut bridge: Bridge,
    channel: u32,
    automatic: bool,
    generation: u64,
    rx: Receiver<Command>,
    state: &ConnectionState<'_>,
) -> (Receiver<Command>, StreamEnd) {
    let tx = state.tx;
    let cancelled = state.cancelled;
    let connected = state.connected;
    let desired = state.desired;
    let report = state.report;
    let initial = *desired.lock().unwrap_or_else(|e| e.into_inner());
    let mut writer = bridge.stdin.take().expect("framed bridge has stdin");
    // Resynchronize with GUI size/focus after offline edits, even when the
    // attached terminal already has an established viewer from another app.
    let mut initial_size = Vec::with_capacity(4);
    initial_size.extend_from_slice(&initial.cols.to_le_bytes());
    initial_size.extend_from_slice(&initial.rows.to_le_bytes());
    let _ = write_frame(&mut writer, channel, RESIZE, &initial_size);
    let _ = write_frame(&mut writer, channel, FOCUS, &[u8::from(initial.focused)]);
    let child = bridge.child.clone();
    let output_writer = std::thread::spawn(move || {
        while let Ok(command) = rx.recv() {
            let result = match command {
                Command::Input(bytes, is_automatic) => {
                    let kind = if is_automatic && automatic { AUTOMATIC_INPUT } else { DATA };
                    let mut result = Ok(());
                    for chunk in bytes.chunks(64 * 1024) {
                        result = write_frame(&mut writer, channel, kind, chunk);
                        if result.is_err() { break; }
                    }
                    result
                }
                Command::Credit(source_generation, amount) => {
                    if source_generation != generation { continue; }
                    write_frame(&mut writer, channel, CREDIT, &amount.to_le_bytes())
                }
                Command::Resize(cols, rows) => {
                    let mut payload = Vec::with_capacity(4);
                    payload.extend_from_slice(&cols.to_le_bytes());
                    payload.extend_from_slice(&rows.to_le_bytes());
                    write_frame(&mut writer, channel, RESIZE, &payload)
                }
                Command::Focus(focused) =>
                    write_frame(&mut writer, channel, FOCUS, &[u8::from(focused)]),
                Command::Reconnect | Command::Detach => {
                    // Notify core that the *viewer* is detaching, then
                    // terminate only this hc connect subprocess so a blocked
                    // reader is guaranteed to wake up. Herdr owns the PTY.
                    let _ = write_frame(&mut writer, channel, CLOSE, &[]);
                    let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
                    break;
                }
                Command::StopWriter => break,
            };
            if result.is_err() {
                let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
                break;
            }
        }
        rx
    });

    let outcome = loop {
        match read_frame(&mut bridge.stdout) {
            Ok(frame) if frame.channel == channel && frame.kind == DATA => {
                report(TerminalEvent::Output { generation, bytes: frame.payload });
            }
            Ok(frame) if frame.channel == channel && frame.kind == CLOSE => {
                if terminal_finished(&frame.payload) {
                    break StreamEnd::Finished;
                }
                break StreamEnd::Lost(
                    if frame.payload.is_empty() {
                        "Herdr closed this viewer channel".into()
                    } else {
                        String::from_utf8_lossy(&frame.payload).into_owned()
                    },
                );
            }
            Ok(_) => {}
            Err(error) => break StreamEnd::Lost(error.to_string()),
        }
    };
    connected.store(false, Ordering::SeqCst);
    // Force any blocked read/write of hc connect to return before joining
    // the writer. Never hold a child lock across join.
    bridge.terminate();
    let _ = tx.send(Command::StopWriter);
    let rx = output_writer.join().expect("framed writer thread panicked");
    if cancelled.load(Ordering::SeqCst) {
        (rx, StreamEnd::Lost("viewer closed".into()))
    } else {
        (rx, outcome)
    }
}

fn serve(
    cwd: PathBuf,
    resume: Option<String>,
    mut rx: Receiver<Command>,
    state: &ConnectionState<'_>,
) -> Result<()> {
    let cancelled = state.cancelled;
    let connected = state.connected;
    let desired = state.desired;
    let report = state.report;
    let executable = std::env::var_os("GENTLE_SHELL_HERDR_HC")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| OsString::from("hc"));
    // Resolve once. NEVER call terminal.run again after this point.
    let address = match resume.or_else(|| {
        std::env::var("GENTLE_SHELL_HERDR_TERMINAL")
            .ok()
            .filter(|v| !v.trim().is_empty())
    }) {
        Some(address) => address,
        None => {
            if cancelled.load(Ordering::SeqCst) { return Ok(()); }
            let mut client = HerdrClient::connect_with_executable(&executable)?;
            let size = *desired.lock().unwrap_or_else(|e| e.into_inner());
            let run = client.call("terminal.run", json!({
                "cwd": cwd.to_string_lossy(),
                "no_parent": true,
                "wait": false,
                "size": {"cols": size.cols, "rows": size.rows},
                "env": {"TERM": "xterm-256color", "COLORTERM": "truecolor",
                        "TERM_PROGRAM": "GentleShell"}
            }))?;
            run["address"]
                .as_str()
                .ok_or_else(|| Error::Protocol("terminal.run returned no address".into()))?
                .to_owned()
        }
    };
    if cancelled.load(Ordering::SeqCst) { return Ok(()); }
    report(TerminalEvent::Assigned(address.clone()));

    let mut failures = 0u32;
    let mut generation = 0u64;
    loop {
        if !discard_offline(&rx, &cancelled) { return Ok(()); }
        let size = *desired.lock().unwrap_or_else(|e| e.into_inner());
        let attempted = Bridge::connect(&executable).and_then(|(mut bridge, feature)| {
            let (channel, automatic) = bridge.attach(&address, size.cols, size.rows, generation > 0)?;
            Ok((bridge, feature, channel, automatic))
        });
        let error = match attempted {
            Ok((bridge, feature, channel, automatic)) => {
                failures = 0;
                let recovered = generation > 0;
                generation += 1;
                connected.store(true, Ordering::SeqCst);
                report(TerminalEvent::Attached { address: address.clone(), recovered });
                let (returned, end) = stream(
                    bridge, channel, feature && automatic, generation, rx, state,
                );
                rx = returned;
                if cancelled.load(Ordering::SeqCst) { return Ok(()); }
                match end {
                    StreamEnd::Finished => {
                        report(TerminalEvent::Closed);
                        return Ok(());
                    }
                    StreamEnd::Lost(error) => error,
                }
            }
            Err(error) => error.to_string(),
        };

        failures += 1;
        if failures >= 5 {
            report(TerminalEvent::NeedsReconnect(format!(
                "Herdr {address}: disconnected ({error}). Automatic retries exhausted."
            )));
            if !wait_retry(&rx, &cancelled, None) { return Ok(()); }
            failures = 0;
        } else {
            report(TerminalEvent::Retrying(format!(
                "Herdr {address}: connection lost ({error}). Retry {failures}/4..."
            )));
            if !wait_retry(&rx, &cancelled, Some(retry_delay(failures))) {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::{
        CREDIT, DATA, Command, Error, check_response, discard_offline, encode_frame,
        read_frame, retry_delay, terminal_finished, wait_retry,
    };

    #[test]
    fn final_close_is_not_retried_but_recoverable_close_is() {
        assert!(terminal_finished(br#"{"reason":"exited","exit_code":0}"#));
        assert!(terminal_finished(br#"{"reason":"killed"}"#));
        assert!(!terminal_finished(b""));
        assert!(!terminal_finished(br#"{"reason":"holder_lost"}"#));
        assert!(!terminal_finished(br#"{"reason":"server_stop"}"#));
        assert!(!terminal_finished(br#"{"reason":"stopped_by_restart"}"#));
    }

    #[test]
    fn retry_backoff_is_bounded() {
        assert_eq!(retry_delay(1), Duration::from_secs(1));
        assert_eq!(retry_delay(2), Duration::from_secs(2));
        assert_eq!(retry_delay(3), Duration::from_secs(4));
        assert_eq!(retry_delay(4), Duration::from_secs(8));
        assert_eq!(retry_delay(99), Duration::from_secs(8));
    }

    #[test]
    fn stale_input_and_credit_never_replay_into_next_viewer() {
        let (tx, rx) = mpsc::channel();
        tx.send(Command::Input(b"old command".to_vec(), false)).unwrap();
        tx.send(Command::Credit(1, 4096)).unwrap();
        tx.send(Command::Resize(80, 24)).unwrap();
        let closed = AtomicBool::new(false);
        assert!(discard_offline(&rx, &closed));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn manual_retry_requires_an_explicit_command() {
        let (tx, rx) = mpsc::channel();
        tx.send(Command::Reconnect).unwrap();
        assert!(wait_retry(&rx, &AtomicBool::new(false), None));
    }

    #[test]
    fn manual_retry_is_cancelled_by_detach() {
        let (tx, rx) = mpsc::channel();
        tx.send(Command::Detach).unwrap();
        assert!(!wait_retry(
            &rx,
            &AtomicBool::new(false),
            Some(Duration::from_secs(30)),
        ));
    }

    #[test]
    fn wire_format_is_little_endian_and_round_trips() {
        let bytes = encode_frame(0x01020304, DATA, b"abc").unwrap();
        assert_eq!(bytes, vec![4,3,2,1,0,3,0,0,0,97,98,99]);
        let parsed = read_frame(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(parsed.channel, 0x01020304);
        assert_eq!(parsed.kind, DATA);
        assert_eq!(parsed.payload, b"abc");
    }

    #[test]
    fn parses_credit_frame() {
        let encoded = encode_frame(9, CREDIT, &256_u32.to_le_bytes()).unwrap();
        let frame = read_frame(&mut Cursor::new(encoded)).unwrap();
        assert_eq!(u32::from_le_bytes(frame.payload.try_into().unwrap()), 256);
    }

    #[test]
    fn oversized_frame_is_rejected_without_allocation() {
        let mut header = [0u8; 9];
        header[5..9].copy_from_slice(&(16_777_217_u32).to_le_bytes());
        assert!(matches!(read_frame(&mut Cursor::new(header)), Err(Error::Protocol(_))));
    }

    #[test]
    fn rpc_response_matches_request_id() {
        assert_eq!(check_response(json!({"jsonrpc":"2.0","id":2,"result":{"channel":3}}), 2)
            .unwrap()["channel"], 3);
        assert!(matches!(
            check_response(json!({"jsonrpc":"2.0","id":1,"result":{}}), 2),
            Err(Error::Protocol(_))
        ));
    }
}
