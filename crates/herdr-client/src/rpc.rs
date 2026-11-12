//! Line-delimited JSON-RPC 2.0 transport used by hc connect in plain mode.
//!
//! Framed terminal attachments are deliberately outside this stage. Keep the
//! transport isolated so the same client can negotiate framed mode later.

use std::fmt;
use std::io::{self, BufRead, Read, Write};

use serde::Deserialize;
use serde_json::{Value, json};

const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct RemoteError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Json(serde_json::Error),
    Protocol(String),
    Remote(RemoteError),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "Herdr I/O error: {e}"),
            Self::Json(e) => write!(f, "Invalid Herdr JSON: {e}"),
            Self::Protocol(message) => write!(f, "Herdr protocol error: {message}"),
            Self::Remote(e) => {
                let kind = e
                    .data
                    .as_ref()
                    .and_then(|v| v.get("kind"))
                    .and_then(Value::as_str);
                match kind {
                    Some(kind) => write!(f, "Herdr error {} ({kind}): {}", e.code, e.message),
                    None => write!(f, "Herdr error {}: {}", e.code, e.message),
                }
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Protocol(_) | Self::Remote(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

/// A single-in-flight JSON-RPC connection. Not thread-safe by design:
/// the caller owns the stream, serializes requests and checks response IDs.
pub(crate) struct JsonRpc<R: BufRead, W: Write> {
    reader: R,
    writer: W,
    next_id: u64,
}

impl<R: BufRead, W: Write> JsonRpc<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            next_id: 1,
        }
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .ok_or_else(|| Error::Protocol("request ID exhausted".into()))?;

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        serde_json::to_writer(&mut self.writer, &request)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;

        // A bounded read prevents an invalid or hostile peer from growing
        // the response buffer without limit.
        let mut line = Vec::new();
        let bytes = (&mut self.reader)
            .take(MAX_RESPONSE_BYTES + 1)
            .read_until(b'\n', &mut line)?;
        if bytes == 0 {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "hc connect closed before replying",
            )));
        }
        if bytes as u64 > MAX_RESPONSE_BYTES {
            return Err(Error::Protocol("JSON-RPC response exceeds 8 MiB".into()));
        }
        if !line.ends_with(b"\n") {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete JSON-RPC response",
            )));
        }

        let response: Value = serde_json::from_slice(&line)?;
        if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(Error::Protocol("missing JSON-RPC 2.0 marker".into()));
        }
        if response.get("id").and_then(Value::as_u64) != Some(id) {
            return Err(Error::Protocol(format!(
                "response ID does not match request {id}"
            )));
        }
        match (response.get("result"), response.get("error")) {
            (Some(result), None) => Ok(result.clone()),
            (None, Some(error)) => Err(Error::Remote(serde_json::from_value(error.clone())?)),
            _ => Err(Error::Protocol(
                "response must contain exactly one of result or error".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::{Value, json};

    use super::{Error, JsonRpc};

    #[test]
    fn sends_valid_json_rpc_and_reads_result() {
        let reply = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"terminals\":[],\"hidden_finished\":0}}\n";
        let mut peer = JsonRpc::new(Cursor::new(reply), Vec::new());

        let result = peer.call("terminal.list", json!({})).unwrap();
        assert_eq!(result["hidden_finished"], 0);
        let request: Value = serde_json::from_slice(&peer.writer).unwrap();
        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["id"], 1);
        assert_eq!(request["method"], "terminal.list");
        assert_eq!(request["params"], json!({}));
    }

    #[test]
    fn increments_ids_for_multiple_calls() {
        let replies = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":null}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":true}\n";
        let mut peer = JsonRpc::new(Cursor::new(replies), Vec::new());
        assert!(peer.call("first", json!({})).unwrap().is_null());
        assert_eq!(peer.call("second", json!({})).unwrap(), json!(true));
        let sent = String::from_utf8(peer.writer).unwrap();
        assert_eq!(sent.lines().count(), 2);
        assert!(sent.lines().nth(1).unwrap().contains(r#""id":2"#));
    }

    #[test]
    fn returns_remote_error_with_kind() {
        let reply = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32000,\"message\":\"no terminal\",\"data\":{\"kind\":\"not_found\"}}}\n";
        let mut peer = JsonRpc::new(Cursor::new(reply), Vec::new());
        match peer.call("terminal.read", json!({})) {
            Err(Error::Remote(e)) => {
                assert_eq!(e.code, -32000);
                assert_eq!(e.data.unwrap()["kind"], "not_found");
            }
            other => panic!("expected remote error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_mismatched_response_id() {
        let reply = b"{\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{}}\n";
        let mut peer = JsonRpc::new(Cursor::new(reply), Vec::new());
        assert!(matches!(peer.call("hello", json!({})), Err(Error::Protocol(_))));
    }

    #[test]
    fn rejects_truncated_response() {
        let mut peer = JsonRpc::new(
            Cursor::new(b"{\"jsonrpc\":\"2.0\",\"id\":1".as_slice()),
            Vec::new(),
        );
        assert!(matches!(peer.call("hello", json!({})), Err(Error::Io(_))));
    }

    #[test]
    fn rejects_result_and_error_together() {
        let reply = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{},\"error\":{\"code\":-1,\"message\":\"bad\"}}\n";
        let mut peer = JsonRpc::new(Cursor::new(reply), Vec::new());
        assert!(matches!(peer.call("hello", json!({})), Err(Error::Protocol(_))));
    }
}
