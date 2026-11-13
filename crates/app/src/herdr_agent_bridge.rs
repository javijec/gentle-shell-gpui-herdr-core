//! GPUI-specific setup for the portable Pi RPC bridge.
//! The remote ARM64 binary runs the same protocol without importing GPUI.
pub fn run(args: Vec<String>) -> i32 {
    crate::features::setup::apply_toolchain_path();
    pi_rpc::set_system_prompt(crate::features::devices::agent_instructions(
        crate::features::setup::devices::prefs(),
    ));
    pi_rpc::herdr_bridge::run(args)
}
