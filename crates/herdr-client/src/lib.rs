//! Client for the herdr-core terminal runtime.
//!
//! The discovery client uses plain-mode JSON-RPC. The terminal module
//! additionally supports framed terminal.attach streams for GPUI panels.
//! Both paths use hc connect on Windows, Linux and macOS.
//!
//! Plain-mode calls are synchronous: run them off the GPUI UI thread.
//! TerminalSession starts a dedicated I/O worker for framed streams.

#![forbid(unsafe_code)]

mod rpc;
pub mod terminal;

use std::ffi::OsStr;
use std::io::BufReader;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde::Deserialize;
use serde_json::{Value, json};

pub use rpc::{Error, RemoteError, Result};

pub const PROTOCOL_GENERATION: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct CoreInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Hello {
    pub generation: u32,
    pub version: String,
    pub core: CoreInfo,
    pub framed: bool,
    pub methods: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
}

/// Terminal records are left as JSON values for now so new herdr-core
/// attributes will not break the stage-1 discovery client.
#[derive(Debug, Deserialize)]
pub struct TerminalList {
    pub terminals: Vec<Value>,
    #[serde(default)]
    pub hidden_finished: u64,
}

/// Owns only the hc connect forwarding process, never the herdr-core daemon.
/// Disconnecting this client does not terminate managed terminals.
struct Bridge(Child);

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A live, synchronous plain-mode client. The underlying process is
/// per-connection, and concurrent callers must use separate clients.
pub struct HerdrClient {
    _bridge: Bridge,
    rpc: rpc::JsonRpc<BufReader<ChildStdout>, ChildStdin>,
    hello: Hello,
}

impl HerdrClient {
    /// Connect through hc on PATH. hc handles Unix sockets/Windows pipes
    /// and starts its core as needed.
    pub fn connect_local() -> Result<Self> {
        Self::connect_with_executable("hc")
    }

    /// Connect using a custom hc executable path (useful for installation
    /// layouts where the binary is not on PATH).
    pub fn connect_with_executable(executable: impl AsRef<OsStr>) -> Result<Self> {
        let mut command = Command::new(executable);
        command.arg("connect");
        Self::connect_command(command)
    }

    /// Connect through an already configured command, such as hc connect.
    /// The command must speak line-delimited JSON-RPC on stdin/stdout.
    ///
    /// This method is blocking, including the initial handshake. Do not
    /// invoke it on GPUI's foreground/UI thread.
    pub fn connect_command(mut command: Command) -> Result<Self> {
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut bridge = Bridge(command.spawn()?);
        let input = bridge
            .0
            .stdin
            .take()
            .ok_or_else(|| Error::Protocol("hc connect has no stdin".into()))?;
        let output = bridge
            .0
            .stdout
            .take()
            .ok_or_else(|| Error::Protocol("hc connect has no stdout".into()))?;

        let mut rpc = rpc::JsonRpc::new(BufReader::new(output), input);
        // Sending framed:false guarantees we remain in the line-based mode;
        // stage 2 will add binary frame handling for terminal.attach.
        let reply = rpc.call(
            "hello",
            json!({"generation": PROTOCOL_GENERATION, "framed": false}),
        )?;
        let hello = parse_hello(reply)?;
        Ok(Self {
            _bridge: bridge,
            rpc,
            hello,
        })
    }

    pub fn hello(&self) -> &Hello {
        &self.hello
    }

    /// List running terminals. This does not attach to, create or kill any.
    pub fn list_terminals(&mut self) -> Result<TerminalList> {
        let reply = self.rpc.call("terminal.list", json!({}))?;
        Ok(serde_json::from_value(reply)?)
    }

    /// Call any plain-mode JSON-RPC method. Not for terminal.attach: that
    /// requires a framed-mode connection, which is a later milestone.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.rpc.call(method, params)
    }
}

fn parse_hello(reply: Value) -> Result<Hello> {
    let hello: Hello = serde_json::from_value(reply)?;
    if hello.generation != PROTOCOL_GENERATION {
        return Err(Error::Protocol(format!(
            "unsupported protocol generation {} (expected {})",
            hello.generation, PROTOCOL_GENERATION
        )));
    }
    if hello.framed {
        return Err(Error::Protocol(
            "hc unexpectedly switched into framed mode".into(),
        ));
    }
    if !hello.methods.iter().any(|name| name == "terminal.list") {
        return Err(Error::Protocol(
            "hc does not advertise terminal.list".into(),
        ));
    }
    Ok(hello)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Error, PROTOCOL_GENERATION, TerminalList, parse_hello};

    #[test]
    fn accepts_generation_one_plain_mode_hello() {
        let hello = parse_hello(json!({
            "generation": PROTOCOL_GENERATION,
            "version": "0.1.19",
            "framed": false,
            "core": {"id": "core-id", "name": "machine"},
            "methods": ["hello", "terminal.list"],
            "features": ["automatic_input"],
        }))
        .unwrap();
        assert_eq!(hello.core.id, "core-id");
        assert_eq!(hello.core.name, "machine");
        assert_eq!(hello.version, "0.1.19");
    }

    #[test]
    fn rejects_unsupported_generation() {
        let result = parse_hello(json!({
            "generation": 2,
            "version": "next",
            "framed": false,
            "core": {"id": "id", "name": "machine"},
            "methods": ["terminal.list"],
        }));
        assert!(matches!(result, Err(Error::Protocol(_))));
    }

    #[test]
    fn parses_terminal_list_without_dropping_unknown_fields() {
        let list: TerminalList = serde_json::from_value(json!({
            "terminals": [{"address": "t1", "state": "live", "extra": 42}],
            "hidden_finished": 3,
        }))
        .unwrap();
        assert_eq!(list.hidden_finished, 3);
        assert_eq!(list.terminals[0]["extra"], 42);
    }
}
