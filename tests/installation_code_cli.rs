use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn sc_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sc"))
}

fn write_test_config(config_dir: &Path) {
    let config = r#"[auth]
session_backend = "file"
signing_key_backend = "file"
"#;
    fs::write(config_dir.join("config.toml"), config).expect("write config");
}

fn temp_config_dir() -> (TempDir, String) {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_test_config(tmp.path());
    let config_dir = tmp.path().to_string_lossy().to_string();
    (tmp, config_dir)
}

fn installation_code_file(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("installation_code.json")
}

fn display_code_from_output(output: &[u8]) -> String {
    let text = std::str::from_utf8(output).expect("utf8 output");
    text.lines()
        .find_map(|line| line.strip_prefix("Installation code: "))
        .expect("installation code line")
        .to_string()
}

#[test]
fn installation_code_reuses_the_same_code() {
    let (_tmp, config_dir) = temp_config_dir();

    let first_output = sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["installation-code"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let second_output = sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["installation-code"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let first_display_code = display_code_from_output(&first_output);
    let second_display_code = display_code_from_output(&second_output);

    assert_eq!(first_display_code, second_display_code);
    assert_eq!(first_display_code.len(), 19);

    let file = installation_code_file(Path::new(&config_dir));
    assert!(file.exists());
    let raw = fs::read_to_string(file).expect("installation code file");
    assert!(raw.contains("\"code\""));
}

#[test]
fn installation_code_json_returns_the_machine_envelope() {
    let (_tmp, config_dir) = temp_config_dir();

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["installation-code", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"ok\":true"))
        .stdout(predicate::str::contains(
            "\"command\":\"installation-code\"",
        ))
        .stdout(predicate::str::contains("\"installation_code\""))
        .stdout(predicate::str::contains("\"display_code\""));
}

#[test]
fn logout_leaves_the_installation_code_file_in_place() {
    let (_tmp, config_dir) = temp_config_dir();

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["installation-code"])
        .assert()
        .success();

    let file = installation_code_file(Path::new(&config_dir));
    assert!(file.exists());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["logout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No active session."));

    assert!(file.exists());
}
