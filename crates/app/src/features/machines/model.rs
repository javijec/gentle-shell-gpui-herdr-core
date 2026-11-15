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
    /// Herdr's authoritative RPC-agent label. Never guess from the title.
    pub gentle_shell: bool,
}

impl Terminal {
    pub fn live(&self) -> bool {
        self.state == "live"
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
        gentle_shell: value["labels"]["gentle-shell"] == "rpc-agent",
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

/// Only list registered machines for the Machines management modal.
/// The workspace's navigation refresh uses discover() for sessions as well.
pub fn discover_registered_machines() -> Result<Snapshot, String> {
    let mut client = client()?;
    let local_name = client.hello().core.name.clone();
    let machines = client.list_machines().map_err(|error| error.to_string())?;
    Ok(Snapshot {
        local_name,
        machines: machines.machines.iter().filter_map(parse_machine).collect(),
        terminals: Vec::new(),
    })
}

/// Request an OS shell on the selected machine. The local Herdr core owns the
/// terminal; a subsequent GPUI panel only attaches to the returned address.
pub fn create_shell(machine: Option<&str>) -> Result<String, String> {
    create_shell_in(machine, None)
}

/// Create a shell in the selected agent's actual project directory. Unlike
/// opening a fresh local PTY, Herdr starts the shell on its owning machine.
pub fn create_shell_in(machine: Option<&str>, cwd: Option<&str>) -> Result<String, String> {
    if let Some(cwd) = cwd
        && (!cwd.starts_with('/') || cwd.contains('\\') || cwd.len() > 2048
            || cwd.chars().any(char::is_control)
            || cwd.split('/').any(|part| part == ".." || part == "."))
    {
        return Err("Invalid remote working directory".into());
    }
    let mut client = client()?;
    let mut params = json!({
        "on": machine,
        "no_parent": true,
        "wait": false,
        "size": {"cols": 80, "rows": 24},
    });
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    let reply = client.call("terminal.run", params)
        .map_err(|e| e.to_string())?;
    reply["address"].as_str()
        .filter(|address| !address.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Herdr returned no terminal address".to_string())
}

/// Build an actual Herdr Core machine.add request. Transport can be an
/// SSH-style host identifier or any supported duplex connection command.
/// The latter uses Herdr's 'via' API and does not require SSH specifically.
fn machine_add_params(
    connection: &str, name: Option<&str>, via: bool,
) -> Result<Value, String> {
    let connection = connection.trim();
    if connection.is_empty() || connection.len() > 1024
        || connection.chars().any(char::is_control)
        || (!via && (connection.starts_with('-')
            || connection.chars().any(char::is_whitespace)))
    {
        return Err(if via {
            "Enter a connection command that can run Herdr Core on the remote machine.".into()
        } else {
            "Enter a valid host or network address.".into()
        });
    }
    let name = name.map(str::trim).filter(|name| !name.is_empty());
    if via && name.is_none() {
        return Err("A machine name is required for a custom Herdr connection.".into());
    }
    if let Some(name) = name
        && (name.len() > 32
            || !name.starts_with(|ch: char| ch.is_ascii_lowercase() || ch.is_ascii_digit())
            || !name.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit()
                || matches!(ch, '-' | '_' | '.')))
    {
        return Err("Herdr machine names must be 1-32 lowercase letters, numbers, - _ or ., beginning with a letter or number.".into());
    }
    let mut params = if via {
        json!({"via": connection})
    } else {
        json!({"target": connection})
    };
    if let Some(name) = name {
        params["name"] = json!(name);
    }
    Ok(params)
}

/// Register another Herdr Core in this one. All networking is managed by
/// Herdr's machine.add; the GUI never launches a shell or an agent.
pub fn add_machine(
    connection: &str, name: Option<&str>, via: bool,
) -> Result<(), String> {
    let params = machine_add_params(connection, name, via)?;
    let mut client = client()?;
    if !client.hello().methods.iter().any(|method| method == "machine.add") {
        return Err("The connected Herdr Core does not support machine.add. Update hc.".into());
    }
    client.call("machine.add", params).map(|_| ()).map_err(|error| {
        let details = error.to_string();
        if details.contains("hc_missing") {
            format!("{details}. Herdr Core must be installed on the remote machine.")
        } else {
            details
        }
    })
}

/// Remove only the registered machine profile. Herdr Core deliberately
/// leaves every remote agent, terminal and saved conversation running.
pub fn remove_machine(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64
        || !name.chars().all(|ch| ch.is_ascii_alphanumeric()
            || matches!(ch, '-' | '_' | '.'))
    {
        return Err("Invalid Herdr machine name.".into());
    }
    let mut client = client()?;
    if !client.hello().methods.iter().any(|method| method == "machine.rm") {
        return Err("The connected Herdr Core does not support machine.rm. Update hc.".into());
    }
    let result = client.call("machine.rm", json!({"machine": name}))
        .map_err(|error| error.to_string())?;
    if result["removed"].as_str() != Some(name) {
        return Err("Herdr did not confirm the removed machine name.".into());
    }
    Ok(())
}

/// Refuse to terminate an arbitrary Herdr terminal or a session whose type
/// changed between rendering Machines and the confirmation dialog.
fn validate_termination(record: &Value, address: &str, is_agent: bool)
    -> Result<bool, String>
{
    let valid_address = !address.is_empty() && address.len() <= 256
        && address.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/')
        });
    if !valid_address || record["address"].as_str() != Some(address) {
        return Err("Herdr session identity changed; refusing to terminate it.".into());
    }
    let actual_agent = record["labels"]["gentle-shell"] == "rpc-agent";
    if is_agent != actual_agent {
        return Err("Herdr session type changed; refresh Machines and retry.".into());
    }
    match record["state"].as_str() {
        Some("live") => Ok(true),
        Some("exited" | "finished" | "killed" | "stopped" | "dead") => Ok(false),
        _ => Err("Herdr session state is unknown; not terminating.".into()),
    }
}

/// Terminate the exact Herdr session, not merely its GPUI viewer. Called
/// asynchronously by Workspace; no UI session is removed on an RPC error.
pub fn terminate_session(address: &str, is_agent: bool) -> Result<(), String> {
    if address.is_empty() || address.len() > 256
        || !address.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/')
        })
    {
        return Err("Invalid Herdr session address.".into());
    }
    let mut client = client()?;
    let record = client.call("terminal.get", json!({"terminal": address}))
        .map_err(|error| error.to_string())?;
    if validate_termination(&record, address, is_agent)? {
        client.call("terminal.kill", json!({"terminal": address}))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::{machine_add_params, parse_machine, parse_terminal, validate_termination};

    #[test]
    fn machine_registration_uses_ssh_target_and_optional_name() {
        let request = machine_add_params(" dietpi@javipi:2222 ", Some("pi-home"), false).unwrap();
        assert_eq!(request, json!({
            "target": "dietpi@javipi:2222", "name": "pi-home"
        }));
        assert_eq!(machine_add_params("workbox", None, false).unwrap(),
            json!({"target":"workbox"}));
        assert_eq!(machine_add_params("ssh://user@10.0.0.7:2222", Some(""), false).unwrap(),
            json!({"target":"ssh://user@10.0.0.7:2222"}));
    }

    #[test]
    fn machine_registration_rejects_empty_or_unsafe_values() {
        for target in ["", " ", "--config", "user@host with spaces", "box\nnext"] {
            assert!(machine_add_params(target, None, false).is_err());
        }
        for name in ["space name", "bad/name", "bad\nname", "UPPERCASE", "-bad"] {
            assert!(machine_add_params("workbox", Some(name), false).is_err());
        }
    }

    #[test]
    fn machine_registration_supports_herdr_via_without_ssh() {
        let request = machine_add_params(
            "wsl.exe -d Ubuntu -e sh -c", Some("wsl"), true,
        ).unwrap();
        assert_eq!(request, json!({
            "via": "wsl.exe -d Ubuntu -e sh -c", "name": "wsl"
        }));
        assert!(machine_add_params("wsl.exe -d Ubuntu", None, true).is_err());
        assert!(machine_add_params("cmd\nthing", Some("win"), true).is_err());
    }

    #[test]
    fn terminate_only_matching_herdr_session_and_type() {
        let agent = json!({
            "address": "javipi/t12", "state": "live",
            "labels":{"gentle-shell":"rpc-agent"}
        });
        assert_eq!(validate_termination(&agent, "javipi/t12", true), Ok(true));
        assert!(validate_termination(&agent, "javipi/t12", false).is_err());
        assert!(validate_termination(&agent, "javipi/t13", true).is_err());
        assert!(validate_termination(&agent, "../t12", true).is_err());
        let finished = json!({
            "address":"javipi/t12", "state":"exited",
            "labels":{"gentle-shell":"rpc-agent"}
        });
        assert_eq!(validate_termination(&finished, "javipi/t12", true), Ok(false));
        let shell = json!({"address":"t8", "state":"live"});
        assert_eq!(validate_termination(&shell, "t8", false), Ok(true));
        assert!(validate_termination(&shell, "t8", true).is_err());
    }

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
        assert!(!local.address.contains('/'));
        assert_eq!(remote.address.split_once('/').map(|(host, _)| host), Some("javipi"));
        assert_eq!(remote.display_title(), "bash");
        assert!(remote.live());
        assert!(!remote.gentle_shell);
    }

    #[test]
    fn detects_gentle_shell_by_authoritative_herdr_label() {
        let agent = parse_terminal(&json!({
            "address":"javipi/t17", "state":"live",
            "title":"Custom title",
            "labels":{"gentle-shell":"rpc-agent"}
        })).unwrap();
        assert!(agent.gentle_shell);
        let shell = parse_terminal(&json!({
            "address":"javipi/t18", "state":"live",
            "title":"Gentle Shell", "labels":{"gentle-shell":"other"}
        })).unwrap();
        assert!(!shell.gentle_shell);
    }

    #[test]
    fn ignores_addresses_and_machine_names_that_are_unusable() {
        assert!(parse_machine(&json!({"name":""})).is_none());
        assert!(parse_machine(&json!({"name":"bad/name"})).is_none());
        assert!(parse_terminal(&json!({"address":""})).is_none());
    }
}
