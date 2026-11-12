//! Blocking Herdr discovery and remote terminal actions.
//! Keep all hc connect JSON-RPC I/O outside the GPUI foreground thread.

use std::ffi::OsString;

use herdr_client::HerdrClient;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    pub name: String,
    pub host: String,
    pub state: String,
    pub target: String,
    pub why: Option<String>,
}

impl Machine {
    pub fn connected(&self) -> bool {
        self.state == "connected"
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    /// Always the exact Herdr address, including `machine/tN` for remote.
    pub address: String,
    pub title: String,
    pub cwd: String,
    pub state: String,
}

impl Terminal {
    pub fn live(&self) -> bool {
        self.state == "live"
    }

    pub fn belongs_to(&self, machine: Option<&str>) -> bool {
        match machine {
            None => !self.address.contains('/'),
            Some(machine) => self.address
                .split_once('/')
                .is_some_and(|(prefix, _)| prefix == machine),
        }
    }

    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            &self.address
        } else {
            &self.title
        }
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub local_name: String,
    pub machines: Vec<Machine>,
    pub terminals: Vec<Terminal>,
}

fn str_field<'a>(value: &'a Value, field: &str) -> &'a str {
    value.get(field).and_then(Value::as_str).unwrap_or("")
}

fn optional_str(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}

pub fn parse_machine(value: &Value) -> Option<Machine> {
    let name = str_field(value, "name").trim();
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some(Machine {
        name: name.to_string(),
        host: str_field(value, "host").to_string(),
        state: str_field(value, "state").to_string(),
        target: str_field(value, "target").to_string(),
        why: optional_str(value, "why"),
    })
}

pub fn parse_terminal(value: &Value) -> Option<Terminal> {
    let address = str_field(value, "address").trim();
    if address.is_empty() {
        return None;
    }
    Some(Terminal {
        address: address.to_string(),
        title: str_field(value, "title").to_string(),
        cwd: str_field(value, "cwd").to_string(),
        state: str_field(value, "state").to_string(),
    })
}

fn client() -> Result<HerdrClient, String> {
    let executable = std::env::var_os("GENTLE_SHELL_HERDR_HC")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("hc"));
    HerdrClient::connect_with_executable(executable).map_err(|e| e.to_string())
}

pub fn discover() -> Result<Snapshot, String> {
    let mut client = client()?;
    let local_name = client.hello().core.name.clone();
    let machines = client.list_machines().map_err(|e| e.to_string())?;
    let terminals = client.list_terminals().map_err(|e| e.to_string())?;
    Ok(Snapshot {
        local_name,
        machines: machines.machines.iter().filter_map(parse_machine).collect(),
        terminals: terminals.terminals.iter().filter_map(parse_terminal).collect(),
    })
}

/// Request an OS shell on the selected machine. The local Herdr core owns the
/// terminal; a subsequent GPUI panel only attaches to the returned address.
pub fn create_shell(machine: Option<&str>) -> Result<String, String> {
    let mut client = client()?;
    let reply = client.call(
        "terminal.run",
        json!({
            "on": machine,
            "no_parent": true,
            "wait": false,
            "size": {"cols": 80, "rows": 24},
        }),
    ).map_err(|e| e.to_string())?;
    reply["address"].as_str()
        .filter(|address| !address.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Herdr returned no terminal address".to_string())
}

pub fn connect_machine(name: &str) -> Result<(), String> {
    let mut client = client()?;
    client.call("machine.connect", json!({"machine": name}))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::{parse_machine, parse_terminal};

    #[test]
    fn parses_connected_and_offline_machines() {
        let online = parse_machine(&json!({
            "name":"javipi", "host":"raspberrypi", "state":"connected",
            "target":"dietpi@javipi", "why":null
        })).unwrap();
        assert_eq!(online.name, "javipi");
        assert!(online.connected());
        let offline = parse_machine(&json!({
            "name":"vps", "state":"unreachable", "why":"ssh unavailable"
        })).unwrap();
        assert!(!offline.connected());
        assert_eq!(offline.why.as_deref(), Some("ssh unavailable"));
    }

    #[test]
    fn separates_local_and_remote_terminals() {
        let local = parse_terminal(&json!({"address":"t7","state":"live"})).unwrap();
        let remote = parse_terminal(&json!({
            "address":"javipi/t10","state":"live","title":"bash"
        })).unwrap();
        assert!(local.belongs_to(None));
        assert!(!local.belongs_to(Some("javipi")));
        assert!(remote.belongs_to(Some("javipi")));
        assert!(!remote.belongs_to(None));
        assert!(!remote.belongs_to(Some("vps")));
        assert_eq!(remote.display_title(), "bash");
        assert!(remote.live());
    }

    #[test]
    fn ignores_addresses_and_machine_names_that_are_unusable() {
        assert!(parse_machine(&json!({"name":""})).is_none());
        assert!(parse_machine(&json!({"name":"bad/name"})).is_none());
        assert!(parse_terminal(&json!({"address":""})).is_none());
    }
}
