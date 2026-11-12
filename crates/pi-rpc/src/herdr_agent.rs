//! Pi RPC over a Herdr-managed local terminal.
//!
//! The child GPUI binary enters --herdr-agent-bridge mode. A tagged base64
//! line protocol carries JSONL over the PTY; regular Pi protocol records
//! never pass directly through terminal emulation. Remote hosts are 4B.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow};
use herdr_client::terminal::{TerminalEvent, TerminalSession};
use serde_json::{Value, json};

use super::herdr_wire::{Decoder, encode_input};
use super::{Incoming, Pending, Record, Response, UiAnswer, parse_record};

const MAX_JSONL: usize = 8 * 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSPORT_TIMEOUT: Duration = Duration::from_secs(12);
const TRANSPORT_PROBE_ID: &str = "gs4a-transport-probe";
type SessionReference = Arc<Mutex<Option<Weak<TerminalSession>>>>;

/// Stage 4B opt-in machine selection until the per-tab UI is added in 4C.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteTarget {
    machine: String,
    cwd: PathBuf,
    bridge: String,
}

fn remote_absolute_path(value: &str) -> bool {
    // Evaluated on Windows, even though this path belongs to Linux ARM64.
    value.starts_with('/')
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && !value.split('/').any(|part| part == ".." || part == ".")
}

fn validate_remote_target(
    machine: Option<&str>,
    cwd: Option<&str>,
    bridge: Option<&str>,
) -> Result<Option<RemoteTarget>> {
    let Some(machine) = machine else {
        anyhow::ensure!(cwd.is_none() && bridge.is_none(),
            "Remote CWD/bridge is set without GENTLE_SHELL_HERDR_AGENT_MACHINE");
        return Ok(None);
    };
    anyhow::ensure!(
        !machine.is_empty() && machine.len() <= 128
            && machine.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "Invalid GENTLE_SHELL_HERDR_AGENT_MACHINE: use the exact registered Herdr machine name"
    );
    let cwd = cwd.ok_or_else(|| anyhow!(
        "Set GENTLE_SHELL_HERDR_REMOTE_CWD to the JaviPi workspace, e.g. /srv/workspaces/JaviPi"
    ))?;
    let bridge = bridge.ok_or_else(|| anyhow!(
        "Set GENTLE_SHELL_HERDR_REMOTE_BRIDGE to the absolute Linux ARM64 bridge executable path"
    ))?;
    anyhow::ensure!(remote_absolute_path(cwd),
        "GENTLE_SHELL_HERDR_REMOTE_CWD must be an absolute Linux path without . or ..");
    anyhow::ensure!(remote_absolute_path(bridge) && bridge != "/",
        "GENTLE_SHELL_HERDR_REMOTE_BRIDGE must be an absolute Linux executable path");
    Ok(Some(RemoteTarget {
        machine: machine.to_string(),
        cwd: PathBuf::from(cwd),
        bridge: bridge.to_string(),
    }))
}

impl RemoteTarget {
    fn from_env() -> Result<Option<Self>> {
        let read = |key: &str| -> Result<Option<String>> {
            match std::env::var(key) {
                Ok(value) => Ok(Some(value)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(_) => Err(anyhow!("{key} must contain UTF-8 characters")),
            }
        };
        let machine = read("GENTLE_SHELL_HERDR_AGENT_MACHINE")?;
        let cwd = read("GENTLE_SHELL_HERDR_REMOTE_CWD")?;
        let bridge = read("GENTLE_SHELL_HERDR_REMOTE_BRIDGE")?;
        validate_remote_target(machine.as_deref(), cwd.as_deref(), bridge.as_deref())
    }
}


/// Verify both directions of Herdr's PTY, not just the bridge's child-pipe
/// handshake. On Windows a terminal may display READY but never deliver input.
fn verify_transport(
    session: &TerminalSession,
    token: &str,
    pending: &Pending,
    ready: &AtomicBool,
) -> Result<()> {
    if !ready.load(Ordering::Acquire) {
        return Err(anyhow!("Herdr bridge disconnected before its transport check"));
    }
    let (tx, rx) = async_channel::bounded(1);
    pending.lock().unwrap().insert(TRANSPORT_PROBE_ID.to_string(), tx);
    let check = (|| {
        let command = json!({"type": "get_state", "id": TRANSPORT_PROBE_ID});
        let mut bytes = serde_json::to_vec(&command)?;
        bytes.push(b'\n');
        for frame in encode_input(token, &bytes) {
            session.input(frame);
        }
        let deadline = Instant::now() + TRANSPORT_TIMEOUT;
        loop {
            match rx.try_recv() {
                Ok(reply) if reply.success => return Ok(()),
                Ok(reply) => return Err(anyhow!(
                    "Herdr RPC transport probe failed: {}",
                    reply.error.unwrap_or_else(|| "get_state returned failure".into())
                )),
                Err(async_channel::TryRecvError::Closed) => {
                    return Err(anyhow!("Herdr RPC transport closed before answering get_state"));
                }
                Err(async_channel::TryRecvError::Empty) => {}
            }
            if !ready.load(Ordering::Acquire) {
                return Err(anyhow!("Herdr RPC transport disconnected during get_state"));
            }
            if Instant::now() >= deadline {
                return Err(anyhow!(
                    "Herdr PTY did not deliver the get_state round trip within 12 seconds"
                ));
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    })();
    pending.lock().unwrap().remove(TRANSPORT_PROBE_ID);
    check
}

/// An agent terminal is ours to stop; never issue a global core shutdown.
fn stop_owned_terminal(address: String) {
    let _ = std::thread::Builder::new().name("herdr-agent-stop".into()).spawn(move || {
        let executable = std::env::var_os("GENTLE_SHELL_HERDR_HC")
            .filter(|x| !x.is_empty()).unwrap_or_else(|| "hc".into());
        if let Ok(mut client) = herdr_client::HerdrClient::connect_with_executable(executable) {
            let _ = client.call("terminal.kill", json!({"terminal":address}));
        }
    });
}

struct IncomingState {
    wire: Decoder,
    jsonl: Vec<u8>,
    stderr_tail: String,
}

impl IncomingState {
    fn new(token: String) -> Self {
        Self {
            wire: Decoder::new(token),
            jsonl: Vec::new(),
            stderr_tail: String::new(),
        }
    }

    /// The outer stream can split a frame anywhere; a Pi record can itself
    /// span hundreds of frames. Echoed input ('I') is never Pi output.
    fn consume(
        &mut self,
        bytes: &[u8],
        pending: &Pending,
        events: &async_channel::Sender<Incoming>,
    ) -> (bool, Option<Option<i32>>) {
        let mut ready = false;
        let mut exited = None;
        for frame in self.wire.push(bytes) {
            match frame.kind {
                'R' => ready = true,
                'X' => {
                    exited = Some(String::from_utf8_lossy(&frame.payload).parse::<i32>().ok());
                }
                'E' => {
                    let line = String::from_utf8_lossy(&frame.payload).into_owned();
                    if self.stderr_tail.len() < 4096 {
                        self.stderr_tail.push_str(&line);
                    }
                    let _ = events.try_send(Incoming::Stderr(line));
                }
                'O' => {
                    for b in frame.payload {
                        if self.jsonl.len() >= MAX_JSONL {
                            self.jsonl.clear();
                            let _ = events.try_send(Incoming::Stderr(
                                "oversized Herdr Pi JSONL record discarded".into(),
                            ));
                            continue;
                        }
                        self.jsonl.push(b);
                        if b == b'\n' {
                            dispatch_record(&self.jsonl, pending, events);
                            self.jsonl.clear();
                        }
                    }
                }
                _ => {}
            }
        }
        (ready, exited)
    }
}

fn dispatch_record(
    line: &[u8],
    pending: &Pending,
    events: &async_channel::Sender<Incoming>,
) {
    let incoming = match parse_record(line) {
        Ok(Record::Response(response)) => {
            let sender = response.id.as_ref().and_then(|id| pending.lock().unwrap().remove(id));
            if let Some(sender) = sender {
                let _ = sender.try_send(response);
            } else if !response.success {
                let _ = events.try_send(Incoming::Stderr(format!(
                    "{} failed: {}",
                    response.command,
                    response.error.unwrap_or_default()
                )));
            }
            return;
        }
        Ok(Record::Ui(ui)) => Incoming::Ui(ui),
        Ok(Record::Event(event)) => Incoming::Event(event),
        Ok(Record::Empty) => return,
        Err(error) => Incoming::Stderr(format!("unparseable Herdr Pi JSONL: {error}")),
    };
    let _ = events.try_send(incoming);
}

/// Owns one Herdr-managed Pi bridge; killing it never stops Herdr Core.
pub struct HerdrAgentProcess {
    session: Arc<TerminalSession>,
    address: Arc<Mutex<Option<String>>>,
    token: String,
    pending: Pending,
    next_id: AtomicU64,
    ready: Arc<AtomicBool>,
    last_failure: Arc<Mutex<Option<String>>>,
    stopping: AtomicBool,
}

impl HerdrAgentProcess {
    /// Run on the background executor. Only this computer is supported in 4A.
    pub fn spawn(
        cwd: PathBuf,
        resume: Option<&Path>,
    ) -> Result<(Arc<Self>, async_channel::Receiver<Incoming>)> {
        let remote = RemoteTarget::from_env()?;
        let ticks = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let token = format!("p{}{:x}", std::process::id(), ticks);
        let (target_cwd, argv, on) = if let Some(remote) = remote {
            // Windows-local session files cannot be resumed on Linux: fail
            // closed instead of leaking the Windows path onto JaviPi.
            anyhow::ensure!(resume.is_none(),
                "Cannot resume a Windows-local session on a remote machine in 4B; open a new agent tab");
            (
                remote.cwd,
                vec![remote.bridge, token.clone()],
                Some(remote.machine),
            )
        } else {
            let executable = std::env::current_exe()
                .context("cannot locate local GPUI bridge executable")?;
            let mut argv = vec![
                executable.to_string_lossy().into_owned(),
                "--herdr-agent-bridge".into(),
                token.clone(),
            ];
            if let Some(path) = resume {
                argv.push("--session".into());
                argv.push(path.to_string_lossy().into_owned());
            }
            (cwd, argv, None)
        };
        let (events, rx) = async_channel::unbounded();
        let (started_tx, started_rx) = mpsc::sync_channel::<Result<()>>(1);
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let address = Arc::new(Mutex::new(None));
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let ready = Arc::new(AtomicBool::new(false));
        let last_failure = Arc::new(Mutex::new(None));
        let incoming = Arc::new(Mutex::new(IncomingState::new(token.clone())));
        let session_ref: SessionReference = Arc::new(Mutex::new(None));

        let session = Arc::new(TerminalSession::spawn_program_at(target_cwd, argv, on, 240, 80, {
            let address = address.clone();
            let pending = pending.clone();
            let ready = ready.clone();
            let last_failure = last_failure.clone();
            let session_ref = session_ref.clone();
            move |event| match event {
                TerminalEvent::Assigned(value) => {
                    *address.lock().unwrap() = Some(value);
                }
                TerminalEvent::Attached { .. } => {}
                TerminalEvent::Output { generation, bytes } => {
                    if let Some(session) = session_ref.lock().unwrap().as_ref().and_then(Weak::upgrade) {
                        session.consumed(generation, bytes.len());
                    }
                    let (started, exit, stderr_tail) = {
                        let mut state = incoming.lock().unwrap();
                        let (started, exit) = state.consume(&bytes, &pending, &events);
                        (started, exit, state.stderr_tail.clone())
                    };
                    if started {
                        *last_failure.lock().unwrap() = None;
                        ready.store(true, Ordering::Release);
                        if let Some(tx) = started_tx.lock().unwrap().take() {
                            let _ = tx.send(Ok(()));
                        }
                    }
                    if let Some(code) = exit {
                        *last_failure.lock().unwrap() = Some(format!(
                            "Gentle Shell agent exited (code {code:?}): {stderr_tail}"
                        ));
                        ready.store(false, Ordering::Release);
                        pending.lock().unwrap().clear();
                        if let Some(tx) = started_tx.lock().unwrap().take() {
                            let _ = tx.send(Err(anyhow!(
                                "Gentle Shell could not start under Herdr (code {code:?}): {stderr_tail}"
                            )));
                        }
                        let _ = events.try_send(Incoming::Exited(code));
                    }
                }
                TerminalEvent::Retrying(error)
                | TerminalEvent::NeedsReconnect(error)
                | TerminalEvent::Failed(error) => {
                    *last_failure.lock().unwrap() = Some(error.clone());
                    ready.store(false, Ordering::Release);
                    pending.lock().unwrap().clear();
                    let _ = events.try_send(Incoming::Stderr(error.clone()));
                    if let Some(tx) = started_tx.lock().unwrap().take() {
                        let _ = tx.send(Err(anyhow!(error)));
                    } else {
                        // 4A fails closed. Automatic Pi RPC recovery is 4D.
                        let _ = events.try_send(Incoming::Exited(None));
                    }
                }
                TerminalEvent::Closed => {
                    let details = incoming.lock().unwrap().stderr_tail.clone();
                    *last_failure.lock().unwrap() = Some(format!(
                        "Herdr agent terminal closed: {details}"
                    ));
                    ready.store(false, Ordering::Release);
                    pending.lock().unwrap().clear();
                    if let Some(tx) = started_tx.lock().unwrap().take() {
                        let details = incoming.lock().unwrap().stderr_tail.clone();
                        let _ = tx.send(Err(anyhow!(
                            "Herdr bridge closed before Pi became ready: {details}"
                        )));
                    }
                    let _ = events.try_send(Incoming::Exited(None));
                }
            }
        }));
        *session_ref.lock().unwrap() = Some(Arc::downgrade(&session));
        let started = match started_rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err(anyhow!("Herdr RPC bridge did not become ready within 30 seconds")),
        };
        // Bridge READY is insufficient: test a real RPC command through
        // terminal input + decoded output before SessionView refreshes models.
        let verified = started.and_then(|()| verify_transport(
            &session, &token, &pending, &ready,
        ));
        if let Err(error) = verified {
            if let Some(address) = address.lock().unwrap().clone() {
                stop_owned_terminal(address);
            }
            return Err(error);
        }
        Ok((
            Arc::new(Self {
                session,
                address,
                token,
                pending,
                next_id: AtomicU64::new(1),
                ready,
                last_failure,
                stopping: AtomicBool::new(false),
            }),
            rx,
        ))
    }

    fn write(&self, value: &Value) -> Result<()> {
        if !self.ready.load(Ordering::Acquire) {
            let details = self.last_failure.lock().unwrap().clone()
                .unwrap_or_else(|| "Herdr RPC bridge is not connected".into());
            return Err(anyhow!(details));
        }
        let mut json = serde_json::to_vec(value)?;
        json.push(b'\n');
        // This first delivery is optimized for ordinary text turns. Reject
        // oversized binary attachments instead of queueing tens of thousands
        // of PTY frames synchronously on the GPUI foreground thread.
        anyhow::ensure!(json.len() <= 1024 * 1024,
            "Herdr RPC 4A request exceeds the 1 MiB foreground limit");
        for frame in encode_input(&self.token, &json) {
            self.session.input(frame);
        }
        Ok(())
    }

    pub fn request(&self, mut command: Value) -> impl Future<Output = Result<Response>> + use<> {
        let result = (|| {
            let id = format!("gs-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
            let object = command.as_object_mut()
                .ok_or_else(|| anyhow!("RPC command must be a JSON object"))?;
            object.insert("id".into(), Value::String(id.clone()));
            let (tx, rx) = async_channel::bounded(1);
            self.pending.lock().unwrap().insert(id.clone(), tx);
            if let Err(error) = self.write(&command) {
                self.pending.lock().unwrap().remove(&id);
                return Err(error);
            }
            Ok(rx)
        })();
        let last_failure = self.last_failure.clone();
        async move {
            let response = result?.recv().await.map_err(|_| {
                let details = last_failure.lock().unwrap().clone()
                    .unwrap_or_else(|| "Herdr RPC bridge exited before replying".into());
                anyhow!(details)
            })?;
            if response.success {
                Ok(response)
            } else {
                Err(anyhow!(response.error.unwrap_or_else(|| "command failed".into())))
            }
        }
    }

    pub fn respond_ui(&self, id: &str, answer: UiAnswer) -> Result<()> {
        let value = match answer {
            UiAnswer::Value(value) => json!({"type":"extension_ui_response","id":id,"value":value}),
            UiAnswer::Confirmed(confirmed) => {
                json!({"type":"extension_ui_response","id":id,"confirmed":confirmed})
            }
            UiAnswer::Cancelled => json!({"type":"extension_ui_response","id":id,"cancelled":true}),
        };
        self.write(&value)
    }

    /// 4A stops its owned Herdr process on close; persistent restart is 4D.
    pub fn shutdown(&self, _grace: Duration) {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return;
        }
        self.ready.store(false, Ordering::Release);
        self.pending.lock().unwrap().clear();
        if let Some(address) = self.address.lock().unwrap().clone() {
            stop_owned_terminal(address);
        }
    }
}

impl Drop for HerdrAgentProcess {
    fn drop(&mut self) {
        self.shutdown(Duration::from_secs(3));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{Incoming, IncomingState, Pending, validate_remote_target};
    use crate::herdr_wire::encode;

    #[test]
    fn valid_remote_target_has_linux_paths_and_exact_machine_name() {
        let target = validate_remote_target(
            Some("javipi"),
            Some("/srv/workspaces/JaviPi"),
            Some("/home/dietpi/.local/bin/gentle-shell-herdr-bridge"),
        ).unwrap().unwrap();
        assert_eq!(target.machine, "javipi");
        assert_eq!(target.cwd.to_str(), Some("/srv/workspaces/JaviPi"));
        assert_eq!(target.bridge, "/home/dietpi/.local/bin/gentle-shell-herdr-bridge");
        assert_eq!(validate_remote_target(None, None, None).unwrap(), None);
    }

    #[test]
    fn remote_selection_fails_closed_on_missing_or_windows_paths() {
        assert!(validate_remote_target(Some("javipi"), None, Some("/bin/bridge")).is_err());
        assert!(validate_remote_target(Some("javipi"), Some("/srv/JaviPi"), None).is_err());
        assert!(validate_remote_target(Some("bad/name"), Some("/srv"), Some("/bin/bridge")).is_err());
        assert!(validate_remote_target(Some("javipi"), Some(r"D:\\Programacion"), Some("/bin/bridge")).is_err());
        assert!(validate_remote_target(Some("javipi"), Some("/srv/../tmp"), Some("/bin/bridge")).is_err());
        assert!(validate_remote_target(Some("javipi"), Some("/srv/JaviPi"), Some("bridge")).is_err());
        assert!(validate_remote_target(None, Some("/srv/JaviPi"), None).is_err());
    }

    #[test]
    fn streamed_events_and_ui_requests_keep_original_types() {
        let pending: Pending = Arc::new(Mutex::new(Default::default()));
        let (events, rx) = async_channel::unbounded();
        let mut parser = IncomingState::new("test".into());
        let jsonl = concat!(
            "{\"type\":\"message_update\",\"value\":\"hello\"}\n",
            "{\"type\":\"extension_ui_request\",\"id\":\"ui1\",",
            "\"method\":\"confirm\",\"title\":\"Allow?\",\"message\":\"Run?\"}\n"
        );
        let framed = encode("test", 'O', jsonl.as_bytes()).concat();
        for chunk in framed.chunks(19) {
            parser.consume(chunk, &pending, &events);
        }
        match rx.try_recv().unwrap() {
            Incoming::Event(event) => assert_eq!(event["value"], "hello"),
            _ => panic!("expected message_update event"),
        }
        match rx.try_recv().unwrap() {
            Incoming::Ui(ui) => assert_eq!(ui.id, "ui1"),
            _ => panic!("expected extension UI request"),
        }
    }

    #[test]
    fn echoed_commands_cannot_become_ui_events() {
        let pending: Pending = Arc::new(Mutex::new(Default::default()));
        let (events, rx) = async_channel::unbounded();
        let mut parser = IncomingState::new("test".into());
        let echo = encode("test", 'I', b"{\"type\":\"message_update\"}\n").concat();
        parser.consume(&echo, &pending, &events);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn response_reassembly_correlates_request_ids() {
        let pending: Pending = Arc::new(Mutex::new(Default::default()));
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        pending.lock().unwrap().insert("gs-9".into(), reply_tx);
        let (events, _) = async_channel::unbounded();
        let mut parser = IncomingState::new("test".into());
        let json = br#"{"type":"response","id":"gs-9","command":"get_state","success":true,"data":{"ready":true}}"#;
        let mut input = json.to_vec();
        input.push(b'\n');
        for chunk in encode("test", 'O', &input).concat().chunks(11) {
            parser.consume(chunk, &pending, &events);
        }
        let response = reply_rx.try_recv().unwrap();
        assert_eq!(response.data["ready"], true);
    }
}
