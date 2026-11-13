//! Pi RPC over a Herdr-managed terminal (local or remote).
//!
//! Local agents use the GPUI Rust bridge. Remote agents run a small,
//! dependency-free inline Node.js bridge using Herdr terminal.run argv;
//! no extra binaries or compilation are needed on JaviPi.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow};
use herdr_client::HerdrClient;
use herdr_client::terminal::{TerminalEvent, TerminalSession};
use serde_json::{Value, json};

use super::herdr_wire::{Decoder, encode_input};
use super::{Incoming, Pending, Record, Response, UiAnswer, parse_record};

const MAX_JSONL: usize = 8 * 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
const TRANSPORT_TIMEOUT: Duration = Duration::from_secs(12);
type SessionReference = Arc<Mutex<Option<Weak<TerminalSession>>>>;

/// Inline Node.js bridge is injected as argv via Herdr Core on JaviPi.
/// This contains no host secrets and does not create remote files.
const REMOTE_BRIDGE_JS: &str = include_str!("herdr_remote_bridge.cjs");

/// Stage 4B opt-in machine selection until the per-tab UI is added in 4C.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteTarget {
    machine: String,
    cwd: PathBuf,
    node: String,
}

fn remote_absolute_path(value: &str) -> bool {
    // This is evaluated on Windows, though it refers to a Linux filesystem.
    value.starts_with('/')
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && !value.split('/').any(|part| part == ".." || part == ".")
}

fn validate_remote_target(
    machine: Option<&str>,
    cwd: Option<&str>,
    node: Option<&str>,
) -> Result<Option<RemoteTarget>> {
    let Some(machine) = machine else {
        anyhow::ensure!(cwd.is_none() && node.is_none(),
            "Remote options require GENTLE_SHELL_HERDR_AGENT_MACHINE");
        return Ok(None);
    };
    anyhow::ensure!(
        !machine.is_empty() && machine.len() <= 128
            && machine.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "Invalid GENTLE_SHELL_HERDR_AGENT_MACHINE: use the exact registered Herdr machine name"
    );

    // Safe default for JaviPi. Override for other Linux machines in 4B.
    let cwd = cwd.unwrap_or("/srv/workspaces/JaviPi");
    anyhow::ensure!(remote_absolute_path(cwd),
        "GENTLE_SHELL_HERDR_REMOTE_CWD must be an absolute Linux path without . or ..");

    // Node is already a Gentle Shell runtime dependency. No other remote
    // software, Rust build or installation is required.
    let node = node.unwrap_or("node");
    anyhow::ensure!(
        node == "node" || node == "nodejs"
            || (remote_absolute_path(node) && node != "/"),
        "GENTLE_SHELL_HERDR_REMOTE_NODE must be node, nodejs, or an absolute Linux path"
    );

    Ok(Some(RemoteTarget {
        machine: machine.to_string(),
        cwd: PathBuf::from(cwd),
        node: node.to_string(),
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
        let node = read("GENTLE_SHELL_HERDR_REMOTE_NODE")?;
        validate_remote_target(machine.as_deref(), cwd.as_deref(), node.as_deref())
    }
}

/// Recover only live terminals started with our validated bridge protocol.
/// The token is available in Herdr's original argv, so workspaces persist only
/// the stable address, not a secret or a copy of 5 KiB of JavaScript.
fn token_for_terminal(value: &Value, address: &str) -> Result<String> {
    anyhow::ensure!(
        !address.is_empty() && address.len() <= 256
            && address.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')),
        "Invalid Herdr terminal address"
    );
    anyhow::ensure!(value["address"] == address, "Herdr returned another terminal address");
    anyhow::ensure!(value["state"] == "live", "Herdr RPC agent is no longer running");
    anyhow::ensure!(value["labels"]["gentle-shell"] == "rpc-agent",
        "Terminal is not a Gentle Shell RPC agent");
    let argv = value["argv"].as_array().ok_or_else(|| anyhow!("Herdr terminal has no argv"))?;
    let token = if argv.len() == 4
        && argv[1] == "--eval"
        && argv[2].as_str().is_some_and(|code|
            code.contains("GS4A1|") && code.contains("gs4a-startup"))
    {
        argv[3].as_str()
    } else if argv.len() >= 3 && argv[1] == "--herdr-agent-bridge" {
        argv[2].as_str()
    } else {
        None
    }.ok_or_else(|| anyhow!("Terminal does not use the compatible GPUI Pi RPC bridge"))?;
    anyhow::ensure!(super::herdr_wire::valid_token(token),
        "Herdr agent has an invalid RPC token");
    Ok(token.to_owned())
}

fn core_client() -> Result<HerdrClient> {
    let executable = std::env::var_os("GENTLE_SHELL_HERDR_HC")
        .filter(|path| !path.is_empty()).unwrap_or_else(|| "hc".into());
    HerdrClient::connect_with_executable(executable).map_err(Into::into)
}

fn inspect_existing(address: &str) -> Result<String> {
    let mut client = core_client()?;
    let record = client.call("terminal.get", json!({"terminal": address}))?;
    token_for_terminal(&record, address)
}

/// Discover remote agents from older 4B runs whose addresses were not yet
/// stored in workspaces.json. No terminal is started, attached or killed.
/// The GUI can offer each untracked agent as a recovered tab.
pub fn discover_live_remote_agents() -> Result<Vec<String>> {
    let Some(target) = RemoteTarget::from_env()? else {
        return Ok(Vec::new());
    };
    let mut client = core_client()?;
    let list = client.list_terminals()?;
    Ok(recoverable_addresses(&list.terminals, &target.machine))
}

/// Pure discovery filter: never recover foreign terminals, dead processes or
/// agents owned by a different machine. Stable order is their creation time.
fn recoverable_addresses(records: &[Value], machine: &str) -> Vec<String> {
    let mut agents: Vec<(u64, String)> = records.iter().filter_map(|record| {
        let address = record["address"].as_str()?;
        if address.split_once('/').map(|(prefix, _)| prefix) != Some(machine) {
            return None;
        }
        token_for_terminal(record, address).ok()?;
        let created = record["created_ms"].as_u64().unwrap_or_default();
        Some((created, address.to_string()))
    }).collect();
    agents.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    agents.dedup_by(|a, b| a.1 == b.1);
    agents.into_iter().map(|(_, address)| address).collect()
}

/// Verify both directions of Herdr's PTY, not just the bridge's child-pipe
/// handshake. On Windows a terminal may display READY but never deliver input.
fn verify_transport(
    session: &TerminalSession,
    token: &str,
    pending: &Pending,
    ready: &AtomicBool,
    probe_id: &str,
) -> Result<()> {
    if !ready.load(Ordering::Acquire) {
        return Err(anyhow!("Herdr bridge disconnected before its transport check"));
    }
    let (tx, rx) = async_channel::bounded(1);
    pending.lock().unwrap().insert(probe_id.to_string(), tx);
    let check = (|| {
        let command = json!({"type": "get_state", "id": probe_id});
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
    pending.lock().unwrap().remove(probe_id);
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
    id_prefix: String,
    ready: Arc<AtomicBool>,
    last_failure: Arc<Mutex<Option<String>>>,
    stopping: AtomicBool,
}

impl HerdrAgentProcess {
    /// Run on the background executor. Only this computer is supported in 4A.
    pub fn spawn(
        cwd: PathBuf,
        resume: Option<&Path>,
        existing: Option<&str>,
    ) -> Result<(Arc<Self>, async_channel::Receiver<Incoming>)> {
        // Inspect the existing terminal before attaching: a stale or foreign
        // address must fail closed instead of launching another Pi agent.
        let existing_token = existing.map(inspect_existing).transpose()?;
        let remote = if existing.is_some() { None } else { RemoteTarget::from_env()? };
        let ticks = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let token = existing_token.unwrap_or_else(|| format!("p{}{:x}", std::process::id(), ticks));
        // A reattached Pi may still answer old requests from a previous GPUI.
        // Never reuse gs-1, gs-2, ... across viewer processes.
        let id_prefix = format!("gs-{}-{:x}", std::process::id(), ticks);
        let (target_cwd, argv, on) = if existing.is_some() {
            (cwd.clone(), Vec::new(), None)
        } else if let Some(remote) = remote {
            // Windows-local session files cannot be resumed on Linux: fail
            // closed instead of leaking the Windows path onto JaviPi.
            anyhow::ensure!(resume.is_none(),
                "Cannot resume a Windows-local session on a remote machine in 4B; open a new agent tab");
            (
                remote.cwd,
                vec![
                    remote.node,
                    "--eval".into(),
                    REMOTE_BRIDGE_JS.into(),
                    token.clone(),
                ],
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
        let reattaching = existing.is_some();
        let remote_start = on.is_some() || existing.is_some_and(|address| address.contains('/'));
        let (events, rx) = async_channel::unbounded();
        let (started_tx, started_rx) = mpsc::sync_channel::<Result<()>>(1);
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let address = Arc::new(Mutex::new(None));
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let ready = Arc::new(AtomicBool::new(false));
        let last_failure = Arc::new(Mutex::new(None));
        let incoming = Arc::new(Mutex::new(IncomingState::new(token.clone())));
        let session_ref: SessionReference = Arc::new(Mutex::new(None));

        let session = Arc::new(TerminalSession::spawn_program_at(
            target_cwd, argv, on, existing.map(str::to_owned), 240, 80, {
            let address = address.clone();
            let pending = pending.clone();
            let ready = ready.clone();
            let last_failure = last_failure.clone();
            let session_ref = session_ref.clone();
            move |event| match event {
                TerminalEvent::Assigned(value) => {
                    *address.lock().unwrap() = Some(value);
                }
                TerminalEvent::Attached { .. } => {
                    // A running bridge emitted READY before GPUI was closed.
                    // After reattaching, validate by get_state round trip;
                    // expecting another READY would hang forever.
                    if reattaching {
                        ready.store(true, Ordering::Release);
                        if let Some(tx) = started_tx.lock().unwrap().take() {
                            let _ = tx.send(Ok(()));
                        }
                    }
                }
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
        let startup_timeout = if remote_start {
            REMOTE_STARTUP_TIMEOUT
        } else {
            STARTUP_TIMEOUT
        };
        let started = match started_rx.recv_timeout(startup_timeout) {
            Ok(result) => result,
            Err(_) => Err(anyhow!(
                "Herdr {} RPC bridge did not become ready within {} seconds",
                if remote_start { "remote" } else { "local" },
                startup_timeout.as_secs(),
            )),
        };
        // Bridge READY is insufficient: test a real RPC command through
        // terminal input + decoded output before SessionView refreshes models.
        let probe_id = format!("{id_prefix}-probe");
        let verified = started.and_then(|()| verify_transport(
            &session, &token, &pending, &ready, &probe_id,
        ));
        if let Err(error) = verified {
            // Never kill an *existing* agent just because this viewer failed
            // to reconnect; the owner can retry later.
            if !reattaching {
                if let Some(address) = address.lock().unwrap().clone() {
                    stop_owned_terminal(address);
                }
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
                id_prefix,
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
            let id = format!("{}-{}", self.id_prefix, self.next_id.fetch_add(1, Ordering::Relaxed));
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

    /// A tab can be explicitly closed before its saved agent was reattached.
    /// Validate ownership in a worker before terminating that exact terminal.
    pub fn terminate_saved(address: String) {
        let _ = std::thread::Builder::new()
            .name("herdr-agent-close-saved".into())
            .spawn(move || {
                if inspect_existing(&address).is_ok() {
                    stop_owned_terminal(address);
                }
            });
    }

    /// Stable address stored in the workspace for process-preserving restart.
    pub fn address(&self) -> Option<String> {
        self.address.lock().unwrap().clone()
    }

    /// On app quit keep remote agents alive; dropping TerminalSession detaches
    /// its viewer, and setting stopping prevents Drop from issuing terminal.kill.
    /// Explicit tab close still uses shutdown and kills its owned terminal.
    pub fn park_for_app_quit(&self) -> bool {
        if self.address().as_deref().is_some_and(|address| address.contains('/')) {
            self.stopping.store(true, Ordering::Release);
            self.ready.store(false, Ordering::Release);
            self.pending.lock().unwrap().clear();
            true
        } else {
            false
        }
    }

    /// Explicit close/restart terminates the managed agent, never Herdr Core.
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

    use super::{
        Incoming, IncomingState, Pending, REMOTE_BRIDGE_JS, recoverable_addresses,
        token_for_terminal, validate_remote_target,
    };
    use crate::herdr_wire::encode;
    use serde_json::json;

    #[test]
    fn remote_target_needs_no_bridge_install_or_rust_compilation() {
        let target = validate_remote_target(Some("javipi"), None, None).unwrap().unwrap();
        assert_eq!(target.machine, "javipi");
        assert_eq!(target.cwd.to_str(), Some("/srv/workspaces/JaviPi"));
        assert_eq!(target.node, "node");
        assert!(REMOTE_BRIDGE_JS.contains("require(\"node:child_process\")"));
        assert!(REMOTE_BRIDGE_JS.contains("GS4A1|"));
        assert!(REMOTE_BRIDGE_JS.contains("gs4a-startup"));
        assert_eq!(validate_remote_target(None, None, None).unwrap(), None);
    }

    #[test]
    fn remote_selection_rejects_windows_paths_and_partial_config() {
        assert!(validate_remote_target(Some("bad/name"), None, None).is_err());
        assert!(validate_remote_target(Some("javipi"), Some(r"D:\\Programacion"), None).is_err());
        assert!(validate_remote_target(Some("javipi"), Some("/srv/../tmp"), None).is_err());
        assert!(validate_remote_target(Some("javipi"), None, Some("sh -c")).is_err());
        assert!(validate_remote_target(Some("javipi"), None, Some(r"C:\\node.exe")).is_err());
        assert!(validate_remote_target(None, Some("/srv/JaviPi"), None).is_err());
        assert!(validate_remote_target(None, None, Some("node")).is_err());
        assert_eq!(
            validate_remote_target(Some("raspi4"), Some("/srv/my-project"), Some("/usr/bin/node"))
                .unwrap().unwrap().node, "/usr/bin/node"
        );
    }

    #[test]
    fn recovery_accepts_only_live_owned_bridge_and_extracts_original_token() {
        let remote = json!({
            "address": "javipi/t42", "state": "live",
            "labels": {"gentle-shell": "rpc-agent"},
            "argv": ["node", "--eval",
                "const name='GS4A1|'; const probe='gs4a-startup';", "p123abc"],
            "created_ms": 7
        });
        let local = json!({
            "address": "t4", "state": "live",
            "labels": {"gentle-shell": "rpc-agent"},
            "argv": ["C:/app/gentle-shell-gpui.exe", "--herdr-agent-bridge", "p0001"],
        });
        assert_eq!(token_for_terminal(&remote, "javipi/t42").unwrap(), "p123abc");
        assert_eq!(token_for_terminal(&local, "t4").unwrap(), "p0001");
        assert!(token_for_terminal(&remote, "javipi/t99").is_err());
        assert!(token_for_terminal(&json!({"address":"javipi/t42","state":"live",
            "labels":{"gentle-shell":"rpc-agent"},
            "argv":["bash","-c","echo hello"]}), "javipi/t42").is_err());
    }

    #[test]
    fn recovered_addresses_are_machine_scoped_and_deterministic() {
        let make = |address: &str, state: &str, time: u64| {
            json!({"address":address,"state":state,
                "labels":{"gentle-shell":"rpc-agent"},
                "created_ms":time,
                "argv":["node","--eval","GS4A1| gs4a-startup", "p123abc"]})
        };
        let candidates = vec![
            make("javipi/t5", "live", 50),
            make("other/t4", "live", 10),
            make("javipi/t3", "finished", 5),
            make("javipi/t1", "live", 1),
            make("javipi/t33", "live", 60),
            json!({"address":"javipi/t2","state":"live","labels":{"gentle-shell":"other"}}),
        ];
        assert_eq!(
            recoverable_addresses(&candidates, "javipi"),
            vec!["javipi/t1", "javipi/t5", "javipi/t33"]
        );
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
