use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

fn write_test_config(config_dir: &tempfile::TempDir) {
    let config = r#"[auth]
session_backend = "file"
signing_key_backend = "file"
"#;
    fs::write(config_dir.path().join("config.toml"), config).expect("write config");
}

#[test]
fn version_flag_prints_cli_version() {
    let config_dir = tempfile::tempdir().expect("tempdir");
    write_test_config(&config_dir);

    Command::new(env!("CARGO_BIN_EXE_sc"))
        .env("SC_CONFIG_DIR", config_dir.path())
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}
