use assert_cmd::Command;
use predicates::prelude::*;
use scalable_cli::dpop::DpopKeyMaterial;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

fn sc_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sc"))
}

fn write_test_config(config_dir: &Path) {
    write_test_config_with_extra(config_dir, "");
}

fn write_test_config_with_extra(config_dir: &Path, extra: &str) {
    let config = r#"[auth]
session_backend = "file"
signing_key_backend = "file"
"#;
    let contents = if extra.trim().is_empty() {
        config.to_string()
    } else {
        format!("{config}\n{extra}\n")
    };
    fs::write(config_dir.join("config.toml"), contents).expect("write config");
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

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl Into<OsString>) -> Self {
        let previous = std::env::var_os(key);
        let value = value.into();
        unsafe {
            std::env::set_var(key, &value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn test_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn ensure_runtime_dpop_key(config_dir: &Path) -> String {
    let _lock = test_env_lock().lock().expect("lock");
    let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir.as_os_str());
    let key = DpopKeyMaterial::load_or_create_default().expect("create dpop key");
    key.jwk_thumbprint().expect("dpop thumbprint")
}

fn write_test_session(config_dir: &Path, access_token: &str) {
    write_test_session_with_options(config_dir, access_token, None, None);
}

fn write_test_session_with_options(
    config_dir: &Path,
    access_token: &str,
    mode: Option<&str>,
    dpop_jwk_thumbprint: Option<&str>,
) {
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
            },
            "dpop_jwk_thumbprint": dpop_jwk_thumbprint,
            "mode": mode,
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

fn capabilities_json(config_dir: &str) -> Value {
    let assert = sc_command()
        .env("SC_CONFIG_DIR", config_dir)
        .args(["capabilities", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(&stdout).expect("machine json envelope")
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
fn broker_context_select_is_allowed_with_local_read_only_session() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());
    write_test_session_with_options(
        tmp.path(),
        "test-access-token",
        Some("local_read_only"),
        None,
    );

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "context",
            "select",
            "--portfolio-id",
            "p-local",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"saved\":true"))
        .stdout(predicate::str::contains("\"portfolio_id\":\"p-local\""));
}

#[test]
fn broker_watchlist_add_json_returns_local_read_only_machine_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());
    let thumbprint = ensure_runtime_dpop_key(tmp.path());
    write_test_session_with_options(
        tmp.path(),
        "test-access-token",
        Some("local_read_only"),
        Some(thumbprint.as_str()),
    );
    write_test_broker_context(tmp.path(), "p-1", "portfolio-1");

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "watchlist",
            "add",
            "--isin",
            "US0378331005",
            "--json",
        ])
        .assert()
        .code(10)
        .stdout(predicate::str::contains(
            "\"command\":\"broker.watchlist.add\"",
        ))
        .stdout(predicate::str::contains("\"code\":\"local_read_only\""))
        .stdout(predicate::str::contains(
            "local read-only mode blocks write operation 'BrokerAddToWatchlist'",
        ));
}

#[test]
fn capabilities_json_reports_disabled_local_trade_controls_when_not_configured() {
    let (_tmp, config_dir) = temp_config_dir();
    let envelope = capabilities_json(&config_dir);
    let controls = &envelope["data"]["local_trade_controls"];

    assert_eq!(envelope["ok"], json!(true));
    assert_eq!(envelope["command"], json!("capabilities"));
    assert_eq!(controls["enabled"], json!(false));
    assert_eq!(controls["isin_controls_active"], json!(false));
    assert_eq!(controls["allowed_isins_configured"], json!(false));
    assert_eq!(controls["denied_isins_configured"], json!(false));
    assert_eq!(controls["max_order_notional_active"], json!(false));
}

#[test]
fn capabilities_json_reports_configured_empty_allowlist_as_active() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config_with_extra(
        tmp.path(),
        r#"[trade_controls]
allowed_isins = []
max_order_notional = "1000"
"#,
    );

    let envelope = capabilities_json(&config_dir);
    let controls = &envelope["data"]["local_trade_controls"];

    assert_eq!(envelope["ok"], json!(true));
    assert_eq!(envelope["command"], json!("capabilities"));
    assert_eq!(controls["enabled"], json!(true));
    assert_eq!(controls["isin_controls_active"], json!(true));
    assert_eq!(controls["allowed_isins_configured"], json!(true));
    assert_eq!(controls["denied_isins_configured"], json!(false));
    assert_eq!(controls["max_order_notional_active"], json!(true));
    assert_eq!(controls["allowed_isins"], json!([]));
    assert_eq!(controls["max_order_notional"], json!("1000"));
}

#[test]
fn broker_trade_buy_json_returns_isin_not_allowed_machine_error_before_session_lookup() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config_with_extra(
        tmp.path(),
        r#"[trade_controls]
allowed_isins = ["IE00B4L5Y983"]
"#,
    );

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "100",
            "--json",
        ])
        .assert()
        .code(10)
        .stdout(predicate::str::contains("\"command\":\"broker.trade.buy\""))
        .stdout(predicate::str::contains(
            "\"code\":\"trade_control_isin_not_allowed\"",
        ))
        .stdout(predicate::str::contains(
            "local trade controls require ISIN 'US0378331005' to be present in allowed_isins",
        ));
}

#[test]
fn broker_trade_buy_json_returns_isin_denied_machine_error_before_session_lookup() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config_with_extra(
        tmp.path(),
        r#"[trade_controls]
allowed_isins = ["US0378331005"]
denied_isins = ["US0378331005"]
"#,
    );

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "100",
            "--json",
        ])
        .assert()
        .code(10)
        .stdout(predicate::str::contains("\"command\":\"broker.trade.buy\""))
        .stdout(predicate::str::contains(
            "\"code\":\"trade_control_isin_denied\"",
        ))
        .stdout(predicate::str::contains(
            "local trade controls deny ISIN 'US0378331005' via denied_isins",
        ));
}

#[test]
fn broker_trade_buy_json_prioritizes_isin_denied_over_allowlist_miss() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config_with_extra(
        tmp.path(),
        r#"[trade_controls]
allowed_isins = ["IE00B4L5Y983"]
denied_isins = ["US0378331005"]
"#,
    );

    sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "trade",
            "buy",
            "--isin",
            "US0378331005",
            "--amount",
            "100",
            "--json",
        ])
        .assert()
        .code(10)
        .stdout(predicate::str::contains("\"command\":\"broker.trade.buy\""))
        .stdout(predicate::str::contains(
            "\"code\":\"trade_control_isin_denied\"",
        ))
        .stdout(predicate::str::contains(
            "local trade controls deny ISIN 'US0378331005' via denied_isins",
        ));
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
fn broker_chart_json_without_session_returns_no_session_code() {
    assert_json_no_session(
        &[
            "broker",
            "chart",
            "--isin",
            "US0378331005",
            "--timeframe",
            "1m",
            "--json",
        ],
        "broker.chart",
    );
}

#[test]
fn broker_quote_json_invalid_isin_returns_broker_input_invalid_code() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());
    let thumbprint = ensure_runtime_dpop_key(tmp.path());
    write_test_session_with_options(
        tmp.path(),
        "test-access-token",
        None,
        Some(thumbprint.as_str()),
    );

    let assert = sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "quote",
            "--portfolio-id",
            "portfolio-1",
            "--isin",
            "US0378331006",
            "--json",
        ])
        .assert()
        .failure()
        .code(10);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let envelope: Value = serde_json::from_str(&stdout).expect("machine json envelope");
    assert_eq!(envelope["command"], json!("broker.quote"));
    assert_eq!(envelope["error"]["code"], json!("broker_input_invalid"));
    assert_eq!(
        envelope["error"]["message"],
        json!("Broker input invalid: field 'isin' must be a valid ISIN")
    );
}

#[test]
fn broker_chart_json_invalid_isin_returns_broker_input_invalid_code() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());
    let thumbprint = ensure_runtime_dpop_key(tmp.path());
    write_test_session_with_options(
        tmp.path(),
        "test-access-token",
        None,
        Some(thumbprint.as_str()),
    );

    let assert = sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "chart",
            "--isin",
            "US0378331006",
            "--timeframe",
            "1m",
            "--json",
        ])
        .assert()
        .failure()
        .code(10);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let envelope: Value = serde_json::from_str(&stdout).expect("machine json envelope");
    assert_eq!(envelope["command"], json!("broker.chart"));
    assert_eq!(envelope["error"]["code"], json!("broker_input_invalid"));
    assert_eq!(
        envelope["error"]["message"],
        json!("Broker input invalid: field 'isin' must be a valid ISIN")
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
fn broker_derivatives_search_json_invalid_underlying_returns_broker_input_invalid_code() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().to_string_lossy().to_string();
    write_test_config(tmp.path());
    write_test_session(tmp.path(), "test-access-token");

    let assert = sc_command()
        .env("SC_CONFIG_DIR", &config_dir)
        .args([
            "broker",
            "derivatives",
            "search",
            "--portfolio-id",
            "portfolio-1",
            "--underlying",
            "US0378331006",
            "--type",
            "warrant",
            "--strategy",
            "call",
            "--json",
        ])
        .assert()
        .failure()
        .code(10);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let envelope: Value = serde_json::from_str(&stdout).expect("machine json envelope");
    assert_eq!(envelope["command"], json!("broker.derivatives.search"));
    assert_eq!(envelope["error"]["code"], json!("broker_input_invalid"));
    assert_eq!(
        envelope["error"]["message"],
        json!("Broker input invalid: field 'underlying' must be a valid ISIN")
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
