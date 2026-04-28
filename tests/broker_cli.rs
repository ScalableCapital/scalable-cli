use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
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

fn current_session_env() -> &'static str {
    #[cfg(feature = "channel-prod")]
    {
        "prod"
    }
    #[cfg(not(feature = "channel-prod"))]
    {
        "dev"
    }
}

fn write_test_session(config_dir: &Path, access_token: &str) {
    fs::write(
        config_dir.join("session.json"),
        json!({
            "env": current_session_env(),
            "session": {
                "access_token": access_token,
                "refresh_token": null,
                "id_token": null,
                "expires_at": 9_999_999_999_i64,
                "person_id": "p-1",
                "source": "device_code"
            }
        })
        .to_string(),
    )
    .expect("write session");
}

fn write_test_broker_context(config_dir: &Path, account_id: &str, portfolio_id: &str) {
    fs::write(
        config_dir.join("broker_context.json"),
        json!({
            "account_id": account_id,
            "portfolio_id": portfolio_id,
        })
        .to_string(),
    )
    .expect("write broker context");
}

fn write_test_trade_attempt(config_dir: &Path) {
    fs::write(config_dir.join("trade_attempt.json"), "{}").expect("write trade attempt");
}

fn write_test_trade_confirmation(config_dir: &Path) {
    fs::write(config_dir.join("trade_confirmation.json"), "{}").expect("write trade confirmation");
}

fn temp_config_dir() -> (TempDir, String) {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_test_config(tmp.path());
    let config_dir = tmp.path().to_string_lossy().to_string();
    (tmp, config_dir)
}

fn assert_json_no_session(args: &[&str], command: &str) {
    let (_tmp, config_dir) = temp_config_dir();

    let assert = sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(args)
        .assert()
        .failure();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let envelope: Value = serde_json::from_str(&stdout).expect("machine json envelope");
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["command"], json!(command));
    assert_eq!(envelope["error"]["code"], json!("no_session"));
    assert_eq!(
        envelope["hints"],
        json!(["Run 'sc login' first to create a session."])
    );
}

#[test]
fn help_shows_broker() {
    let (_tmp, config_dir) = temp_config_dir();

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("broker"));
}

#[test]
fn broker_transactions_help_lists_valid_filter_values() {
    let (_tmp, config_dir) = temp_config_dir();

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["broker", "transactions", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("CASH_TRANSFER_IN"))
        .stdout(predicate::str::contains("REINVESTMENT_POCKET_MONEY"))
        .stdout(predicate::str::contains("CANCEL_REQUESTED"))
        .stdout(predicate::str::contains("CONFIRMED"));
}

#[test]
fn broker_context_select_and_show_roundtrip_json() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());
    write_test_session(tmp.path(), "test-access-token");

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "context",
            "select",
            "--portfolio-id",
            "p1",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"saved\":true"));

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["broker", "context", "show", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"account_id\":\"p-1\""))
        .stdout(predicate::str::contains("\"portfolio_id\":\"p1\""));
}

#[test]
fn logout_human_removes_broker_context_with_active_session() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_path_buf();
    write_test_config(&config_dir);
    write_test_session(&config_dir, "test-access-token");
    write_test_broker_context(&config_dir, "p-1", "portfolio-1");
    write_test_trade_attempt(&config_dir);
    write_test_trade_confirmation(&config_dir);
    let context_file = config_dir.join("broker_context.json");
    let attempt_file = config_dir.join("trade_attempt.json");
    let confirmation_file = config_dir.join("trade_confirmation.json");

    assert!(context_file.exists());
    assert!(attempt_file.exists());
    assert!(confirmation_file.exists());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["logout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Logged out."));

    assert!(!context_file.exists());
    assert!(!attempt_file.exists());
    assert!(!confirmation_file.exists());
}

#[test]
fn logout_json_removes_broker_context_with_active_session() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_path_buf();
    write_test_config(&config_dir);
    write_test_session(&config_dir, "test-access-token");
    write_test_broker_context(&config_dir, "p-1", "portfolio-1");
    write_test_trade_attempt(&config_dir);
    write_test_trade_confirmation(&config_dir);
    let context_file = config_dir.join("broker_context.json");
    let attempt_file = config_dir.join("trade_attempt.json");
    let confirmation_file = config_dir.join("trade_confirmation.json");

    assert!(context_file.exists());
    assert!(attempt_file.exists());
    assert!(confirmation_file.exists());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["logout", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"logged_out\":true"));

    assert!(!context_file.exists());
    assert!(!attempt_file.exists());
    assert!(!confirmation_file.exists());
}

#[test]
fn logout_human_without_session_still_removes_broker_context() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_path_buf();
    write_test_config(&config_dir);
    write_test_broker_context(&config_dir, "p-1", "portfolio-1");
    write_test_trade_attempt(&config_dir);
    write_test_trade_confirmation(&config_dir);
    let context_file = config_dir.join("broker_context.json");
    let attempt_file = config_dir.join("trade_attempt.json");
    let confirmation_file = config_dir.join("trade_confirmation.json");

    assert!(context_file.exists());
    assert!(attempt_file.exists());
    assert!(confirmation_file.exists());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["logout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No active session."));

    assert!(!context_file.exists());
    assert!(!attempt_file.exists());
    assert!(!confirmation_file.exists());
}

#[test]
fn logout_json_without_session_still_removes_broker_context() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_path_buf();
    write_test_config(&config_dir);
    write_test_broker_context(&config_dir, "p-1", "portfolio-1");
    write_test_trade_attempt(&config_dir);
    write_test_trade_confirmation(&config_dir);
    let context_file = config_dir.join("broker_context.json");
    let attempt_file = config_dir.join("trade_attempt.json");
    let confirmation_file = config_dir.join("trade_confirmation.json");

    assert!(context_file.exists());
    assert!(attempt_file.exists());
    assert!(confirmation_file.exists());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["logout", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"logged_out\":false"));

    assert!(!context_file.exists());
    assert!(!attempt_file.exists());
    assert!(!confirmation_file.exists());
}

#[test]
fn broker_overview_without_session_fails() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["broker", "overview"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("No active session")
                .or(predicate::str::contains("Platform secure storage failure")),
        );
}

#[test]
fn broker_overview_json_without_session_returns_no_session_code() {
    assert_json_no_session(&["broker", "overview", "--json"], "broker.overview");
}

#[test]
fn broker_analytics_json_without_session_returns_no_session_code() {
    assert_json_no_session(&["broker", "analytics", "--json"], "broker.analytics");
}

#[test]
fn broker_transactions_json_without_session_returns_no_session_code() {
    assert_json_no_session(&["broker", "transactions", "--json"], "broker.transactions");
}

#[test]
fn broker_transaction_details_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "transaction",
            "details",
            "--transaction-id",
            "tx-1",
            "--json",
        ],
        "broker.transaction.details",
    );
}

#[test]
fn broker_transactions_json_invalid_summary_type_filter_returns_broker_input_invalid() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "transactions",
            "--json",
            "--type-filter",
            "CASH_TRANSACTION",
        ])
        .assert()
        .code(10)
        .stdout(predicate::str::contains(
            "\"command\":\"broker.transactions\"",
        ))
        .stdout(predicate::str::contains(
            "\"code\":\"broker_input_invalid\"",
        ))
        .stdout(predicate::str::contains("CASH_TRANSACTION"))
        .stdout(predicate::str::contains("BUY"));
}

#[test]
fn broker_transactions_json_invalid_status_returns_broker_input_invalid() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["broker", "transactions", "--json", "--status", "done"])
        .assert()
        .code(10)
        .stdout(predicate::str::contains(
            "\"command\":\"broker.transactions\"",
        ))
        .stdout(predicate::str::contains(
            "\"code\":\"broker_input_invalid\"",
        ))
        .stdout(predicate::str::contains("DONE"))
        .stdout(predicate::str::contains("FILLED"));
}

#[test]
fn broker_watchlist_add_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "watchlist",
            "add",
            "--isin",
            "US0378331005",
            "--json",
        ],
        "broker.watchlist.add",
    );
}

#[test]
fn broker_quote_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &["broker", "quote", "--isin", "US0378331005", "--json"],
        "broker.quote",
    );
}

#[test]
fn broker_watchlist_add_parent_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "watchlist",
            "--json",
            "add",
            "--isin",
            "US0378331005",
        ],
        "broker.watchlist.add",
    );
}

#[test]
fn broker_watchlist_remove_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "watchlist",
            "remove",
            "--isin",
            "US0378331005",
            "--json",
        ],
        "broker.watchlist.remove",
    );
}

#[test]
fn broker_watchlist_remove_parent_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "watchlist",
            "--json",
            "remove",
            "--isin",
            "US0378331005",
        ],
        "broker.watchlist.remove",
    );
}

#[test]
fn broker_watchlist_add_rejects_list_only_flags_before_subcommand() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "watchlist",
            "--quote-source",
            "CONSOLIDATED",
            "add",
            "--isin",
            "US0378331005",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--quote-source"));
}

#[test]
fn broker_watchlist_add_parent_json_invalid_flags_return_validation_envelope() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "watchlist",
            "--json",
            "--quote-source",
            "CONSOLIDATED",
            "add",
            "--isin",
            "US0378331005",
        ])
        .assert()
        .code(10)
        .stdout(predicate::str::contains(
            "\"command\":\"broker.watchlist.add\"",
        ))
        .stdout(predicate::str::contains(
            "\"code\":\"broker_input_invalid\"",
        ));
}

#[test]
fn broker_watchlist_add_parent_json_blank_quote_source_returns_validation_envelope() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "watchlist",
            "--json",
            "--quote-source",
            "",
            "add",
            "--isin",
            "US0378331005",
        ])
        .assert()
        .code(10)
        .stdout(predicate::str::contains(
            "\"command\":\"broker.watchlist.add\"",
        ))
        .stdout(predicate::str::contains(
            "\"code\":\"broker_input_invalid\"",
        ))
        .stdout(predicate::str::contains("--quote-source"));
}

#[test]
fn broker_savings_plans_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &["broker", "savings-plans", "--json"],
        "broker.savings-plans",
    );
}

#[test]
fn broker_savings_plans_add_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "savings-plans",
            "add",
            "--isin",
            "US0378331005",
            "--amount",
            "100",
            "--json",
        ],
        "broker.savings-plans.add",
    );
}

#[test]
fn broker_savings_plans_remove_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "savings-plans",
            "remove",
            "--isin",
            "US0378331005",
            "--json",
        ],
        "broker.savings-plans.remove",
    );
}

#[test]
fn broker_price_alerts_json_without_session_returns_no_session_code() {
    assert_json_no_session(&["broker", "price-alerts", "--json"], "broker.price-alerts");
}

#[test]
fn broker_price_alerts_add_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "price-alerts",
            "add",
            "--isin",
            "US0378331005",
            "--price",
            "100",
            "--json",
        ],
        "broker.price-alerts.add",
    );
}

#[test]
fn broker_price_alerts_add_parent_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "price-alerts",
            "--json",
            "add",
            "--isin",
            "US0378331005",
            "--price",
            "100",
        ],
        "broker.price-alerts.add",
    );
}

#[test]
fn broker_price_alerts_remove_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "price-alerts",
            "remove",
            "--alert-id",
            "alert-1",
            "--json",
        ],
        "broker.price-alerts.remove",
    );
}

#[test]
fn broker_price_alerts_remove_parent_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "price-alerts",
            "--json",
            "remove",
            "--alert-id",
            "alert-1",
        ],
        "broker.price-alerts.remove",
    );
}

#[test]
fn broker_price_alerts_add_parent_json_invalid_flags_return_broker_input_invalid_envelope() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "price-alerts",
            "--json",
            "--active-only",
            "add",
            "--isin",
            "US0378331005",
            "--price",
            "100",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "\"command\":\"broker.price-alerts.add\"",
        ))
        .stdout(predicate::str::contains(
            "\"code\":\"broker_input_invalid\"",
        ))
        .stdout(predicate::str::contains("--active-only"));
}

#[test]
fn broker_price_alerts_remove_parent_json_invalid_flags_return_broker_input_invalid_envelope() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "price-alerts",
            "--json",
            "--active-only",
            "remove",
            "--alert-id",
            "alert-1",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "\"command\":\"broker.price-alerts.remove\"",
        ))
        .stdout(predicate::str::contains(
            "\"code\":\"broker_input_invalid\"",
        ))
        .stdout(predicate::str::contains("--active-only"));
}

#[test]
fn broker_savings_plans_add_parent_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "savings-plans",
            "--json",
            "add",
            "--isin",
            "US0378331005",
            "--amount",
            "100",
        ],
        "broker.savings-plans.add",
    );
}

#[test]
fn broker_savings_plans_remove_parent_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "savings-plans",
            "--json",
            "remove",
            "--isin",
            "US0378331005",
        ],
        "broker.savings-plans.remove",
    );
}

#[test]
fn broker_trade_buy_without_session_fails() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "500",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("No active session")
                .or(predicate::str::contains("Platform secure storage failure")),
        );
}

#[test]
fn broker_trade_buy_json_without_session_returns_no_session_code() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "500",
            "--json",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("\"command\":\"broker.trade.buy\""))
        .stdout(predicate::str::contains("\"code\":\"no_session\""));
}

#[test]
fn broker_trade_sell_without_session_fails() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "trade",
            "sell",
            "--isin",
            "US0378331005",
            "--shares",
            "1.5",
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("No active session")
                .or(predicate::str::contains("Platform secure storage failure")),
        );
}

#[test]
fn broker_trade_sell_json_without_session_returns_no_session_code() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "trade",
            "sell",
            "--isin",
            "US0378331005",
            "--shares",
            "1.5",
            "--json",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "\"command\":\"broker.trade.sell\"",
        ))
        .stdout(predicate::str::contains("\"code\":\"no_session\""));
}

#[test]
fn broker_trade_cancel_without_session_fails() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["broker", "trade", "cancel", "--order-id", "order-1"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("No active session")
                .or(predicate::str::contains("Platform secure storage failure")),
        );
}

#[test]
fn broker_trade_cancel_json_without_session_returns_no_session_code() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "trade",
            "cancel",
            "--order-id",
            "order-1",
            "--json",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "\"command\":\"broker.trade.cancel\"",
        ))
        .stdout(predicate::str::contains("\"code\":\"no_session\""));
}

#[test]
fn login_rejects_removed_env_flag() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_path_buf();
    write_test_config(&config_dir);

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args(["login", "--env", "dev"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--env"));
}
