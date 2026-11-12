//! Manual smoke test for the stage-1 integration.
//!
//! Run: cargo run --locked -p herdr-client --example probe
//! Optionally specify the hc executable as the first argument.

use std::error::Error;

use herdr_client::HerdrClient;

fn main() -> Result<(), Box<dyn Error>> {
    let mut client = match std::env::args_os().nth(1) {
        Some(path) => HerdrClient::connect_with_executable(path)?,
        None => HerdrClient::connect_local()?,
    };

    let hello = client.hello();
    println!(
        "Connected to {} (core {}, version {}, protocol {})",
        hello.core.name, hello.core.id, hello.version, hello.generation
    );
    let list = client.list_terminals()?;
    println!(
        "{} visible terminals ({} hidden finished)",
        list.terminals.len(),
        list.hidden_finished
    );
    for terminal in list.terminals {
        let address = terminal["address"].as_str().unwrap_or("<unknown>");
        let state = terminal["state"].as_str().unwrap_or("<unknown>");
        println!("{address}: {state}");
    }
    Ok(())
}
