use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};

fn command(config_dir: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sc"));
    command.env("SC_CONFIG_DIR", config_dir);
    command
}

#[test]
fn fixed_term_help_and_capabilities_advertise_the_supported_surface() {
    let tmp = tempfile::tempdir().unwrap();
    command(tmp.path())
        .args(["fixed-term", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list").and(predicate::str::contains("details")));
    command(tmp.path())
        .args(["fixed-term", "details", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--fixed-term-account-id")
                .and(predicate::str::contains("--json")),
        );
    let output = command(tmp.path())
        .args(["capabilities", "--json"])
        .assert()
        .success();
    let envelope: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    let commands = envelope["data"]["commands"].as_array().unwrap();
    assert!(commands.contains(&json!("fixed-term.list")));
    assert!(commands.contains(&json!("fixed-term.details")));
}

#[test]
fn fixed_term_machine_errors_keep_leaf_command_names_and_exit_codes() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("config.toml"),
        "[auth]\nsession_backend = \"file\"\nsigning_key_backend = \"file\"\n",
    )
    .unwrap();
    for (args, leaf, code, exit) in [
        (vec!["list", "--json"], "list", "no_session", 20),
        (
            vec!["details", "--fixed-term-account-id", "ft-1", "--json"],
            "details",
            "no_session",
            20,
        ),
        (vec!["details", "--json"], "details", "invalid_input", 10),
        (
            vec!["details", "--fixed-term-account-id", " ", "--json"],
            "details",
            "invalid_input",
            10,
        ),
    ] {
        let output = command(tmp.path())
            .arg("fixed-term")
            .args(args)
            .assert()
            .code(exit);
        let envelope: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["command"], format!("fixed-term.{leaf}"));
        assert_eq!(envelope["error"]["code"], code);
        assert!(output.get_output().stderr.is_empty());
    }
}
