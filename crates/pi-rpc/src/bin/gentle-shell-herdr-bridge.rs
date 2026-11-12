//! Portable, headless Herdr RPC bridge. Deploy this binary to Linux aarch64
//! (e.g. JaviPi) and run it through Herdr terminal.run with argv.
fn main() {
    std::process::exit(pi_rpc::herdr_bridge::run(
        std::env::args().skip(1).collect(),
    ));
}
