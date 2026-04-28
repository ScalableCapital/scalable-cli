use assert_cmd::Command;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

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

#[test]
fn capabilities_returns_machine_json_envelope() {
    let config_dir = tempfile::tempdir().expect("tempdir");
    write_test_config(config_dir.path());
    let assert = sc_command()
        .args(["capabilities", "--json"])
        .env("SC_CONFIG_DIR", config_dir.path())
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let envelope: Value = serde_json::from_str(&stdout).expect("machine json envelope");
    let data = &envelope["data"];
    let commands = data["commands"]
        .as_array()
        .expect("capabilities commands array");
    let buy_workflow = &data["workflows"]["broker.trade.buy"];

    assert_eq!(envelope["ok"], json!(true));
    assert_eq!(envelope["command"], json!("capabilities"));
    assert!(commands.contains(&json!("installation-code")));
    assert!(commands.contains(&json!("login")));
    assert!(commands.contains(&json!("broker.overview")));
    assert!(commands.contains(&json!("broker.analytics")));
    assert!(commands.contains(&json!("broker.watchlist.add")));
    assert!(commands.contains(&json!("broker.watchlist.remove")));
    assert!(commands.contains(&json!("broker.quote")));
    assert!(commands.contains(&json!("broker.price-alerts.remove")));
    assert!(commands.contains(&json!("broker.trade.buy")));
    assert!(commands.contains(&json!("broker.trade.sell")));
    assert!(commands.contains(&json!("broker.trade.cancel")));
    assert_eq!(data["auth"]["modes"], json!(["device"]));
    assert_eq!(data["auth"]["non_interactive_modes"], json!([]));
    assert_eq!(
        data["command_metadata"]["login"],
        json!({
            "human_only": true,
            "json_supported": false
        })
    );
    assert_eq!(buy_workflow["preferred_output"], json!("json"));
    assert_eq!(
        buy_workflow["phase_1_presentation_requirement"]["must_present_all_information"],
        json!(true)
    );
    assert_eq!(
        buy_workflow["phase_1_presentation_requirement"]["requires_explicit_user_confirmation_between_phases"],
        json!(true)
    );
    assert_eq!(
        buy_workflow["phase_1_presentation_requirement"]["forbid_automatic_phase_2_execution"],
        json!(true)
    );
    assert_eq!(
        buy_workflow["phase_1_presentation_requirement"]["confirmation_must_be_separate_step"],
        json!(true)
    );
    assert!(buy_workflow["phase_1_command_template_json"].is_string());
    assert!(
        buy_workflow["phase_2_command_template_json"]
            .as_str()
            .expect("phase 2 template")
            .contains("--accept-unsuitable")
    );

    let presentation = &buy_workflow["phase_1_presentation_requirement"];
    assert_eq!(
        presentation["must_present_all_information"],
        Value::Bool(true)
    );
    assert_eq!(
        presentation["requires_explicit_user_confirmation_between_phases"],
        Value::Bool(true)
    );
    assert_eq!(
        presentation["forbid_automatic_phase_2_execution"],
        Value::Bool(true)
    );
    assert_eq!(
        presentation["confirmation_must_be_separate_step"],
        Value::Bool(true)
    );
    assert_eq!(
        presentation["raw_json_only_on_user_request"],
        Value::Bool(true)
    );

    let section_order = presentation["section_order"]
        .as_array()
        .expect("section order");
    let price_warnings_idx = section_order
        .iter()
        .position(|item| item.as_str() == Some("price_warnings"))
        .expect("price warnings section");
    let ex_ante_idx = section_order
        .iter()
        .position(|item| item.as_str() == Some("ex_ante_costs"))
        .expect("ex ante section");
    let suitability_idx = section_order
        .iter()
        .position(|item| item.as_str() == Some("suitability"))
        .expect("suitability section");
    let disclosures_idx = section_order
        .iter()
        .position(|item| item.as_str() == Some("regulatory_disclosures"))
        .expect("regulatory disclosures section");
    let document_links_idx = section_order
        .iter()
        .position(|item| item.as_str() == Some("document_links"))
        .expect("document links section");
    let confirmation_idx = section_order
        .iter()
        .position(|item| item.as_str() == Some("confirmation"))
        .expect("confirmation section");
    assert_eq!(price_warnings_idx + 1, ex_ante_idx);
    assert_eq!(suitability_idx, ex_ante_idx + 1);
    assert_eq!(disclosures_idx, suitability_idx + 1);
    assert_eq!(document_links_idx, disclosures_idx + 1);
    assert_eq!(confirmation_idx, document_links_idx + 1);

    let required_leaf_paths = presentation["required_leaf_paths"]
        .as_array()
        .expect("required leaf paths");
    assert!(required_leaf_paths.iter().any(|item| {
        item.as_str() == Some("/result/ex_ante_costs/entryCosts/serviceCosts/amount")
    }));
    assert!(required_leaf_paths.iter().any(|item| {
        item.as_str() == Some("/result/ex_ante_costs/entryCosts/productCosts/amount")
    }));
    assert!(required_leaf_paths.iter().any(|item| {
        item.as_str() == Some("/result/ex_ante_costs/effectOnReturn/initialYearCosts/amount")
    }));
    assert!(required_leaf_paths.iter().any(|item| {
        item.as_str() == Some("/result/ex_ante_costs/effectOnReturn/followingYearsCosts/amount")
    }));
    assert!(
        required_leaf_paths
            .iter()
            .any(|item| item.as_str() == Some("/result/price_warnings/items"))
    );
    assert!(
        required_leaf_paths
            .iter()
            .any(|item| item.as_str() == Some("/result/suitability/status"))
    );
    assert!(
        required_leaf_paths
            .iter()
            .any(|item| { item.as_str() == Some("/result/document_links/client_documents") })
    );
    assert!(required_leaf_paths.iter().any(|item| {
        item.as_str() == Some("/result/regulatory_disclosures/ex_ante_costs_notice")
    }));
    assert_eq!(
        buy_workflow["phase_1_presentation_requirement"]["raw_json_only_on_user_request"],
        json!(true)
    );
}
