//! Host-run integration test over the pure core: no wasm toolchain needed.
//!
//! Replace this with tests that pin down real behavior; it is deliberately
//! shallow so a fresh scaffold still passes `cargo test`.

use {{crate_name | snake_case}}::core::{run, ToolInput};

#[test]
fn run_accepts_default_input() {
    let input = ToolInput::default();
    // TODO: assert the real output once `run` does something.
    let _ = run(&input);
}
