use std::process::Command;

#[test]
fn version_exits_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_umpv"))
        .arg("--version")
        .output()
        .expect("run umpv");
    assert!(output.status.success(), "status: {}", output.status);
}

#[test]
fn unknown_option_is_a_usage_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_umpv"))
        .arg("--definitely-not-an-option")
        .output()
        .expect("run umpv");
    assert_eq!(output.status.code(), Some(2));
}
