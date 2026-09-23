use clap::Parser;
use mockito::{Matcher, Server};
use serde_json::{Value, json};

use crate::cli::Cli;
use crate::config::{
    AppConfig, AuthConfig, DpopKeyBackend, EnvConfig, RuntimeAuthConfig, SessionBackendPreference,
};
use crate::fixed_term_commands::render;
use crate::fixed_term_projections::{NOT_FOUND, project_details, project_list};
use crate::fixed_term_queries::{DETAILS_QUERY, LIST_QUERY};
use crate::machine::classify_error;
use crate::session::{LoginSource, Session, SessionManager, SessionMode, StoredSession};

fn account() -> Value {
    json!({
        "id": "ft-1", "state": "ACTIVE", "configuration": "SINGLE",
        "fixedTerm": 12, "interestRate": "0.0230", "totalAmount": "10000.00",
        "personalizations": {"name": " Festgeld 1 ", "id": "private"},
        "endDate": {"time": "2027-09-01T00:00:00Z"},
        "startDate": {"time": "2026-09-01T00:00:00+02:00"},
        "effectiveInterestRate": "0.0232", "currentAccruedAmount": "1.23",
        "estimatedPayoutAmount": "10230.12345678901234567890", "autoRenewal": false,
        "trusteeBalanceBreakdown": {
            "balances": [{"trusteeName": "Bank", "balance": "10000.00", "id": "private"}],
            "snapshotDateTime": "2026-09-18T08:00:00Z"
        },
        "owners": [{"id": "private"}], "iban": "private", "autoRenewLockPeriodStart": "private"
    })
}

fn details(raw: Value) -> Value {
    json!({"account": {"fixedTermSavingsAccount": raw}})
}

fn list(accounts: Vec<Value>) -> Value {
    json!({"account": {"fixedTermSavingsAccounts": accounts}})
}

#[test]
fn fixed_term_details_expose_only_the_approved_contract_without_decimal_recalculation() {
    assert_eq!(
        project_details(&details(account()), "ft-1").unwrap(),
        json!({
            "fixed_term_account_id": "ft-1", "display_name": "Festgeld 1", "owner_kind": "personal",
            "state": "ACTIVE", "deposit_amount": "10000.00", "currency": "EUR",
            "fixed_term_months": 12, "nominal_interest_rate": "0.0230",
            "maturity_date": "2027-09-01T00:00:00Z",
            "contractual_start_date": "2026-09-01T00:00:00+02:00",
            "effective_interest_rate": "0.0232", "current_accrued_gross_interest": "1.23",
            "estimated_payout_amount": "10230.12345678901234567890", "auto_renewal_enabled": false,
            "allocation": {"balances": [{"trustee_name": "Bank", "amount": "10000.00"}],
                "snapshot_date_time": "2026-09-18T08:00:00Z"}
        })
    );
}

#[test]
fn fixed_term_list_covers_empty_multiple_states_and_filters_unsupported_accounts() {
    assert_eq!(
        project_list(&list(vec![])).unwrap(),
        json!({"accounts": []})
    );
    let mut accounts = Vec::new();
    for state in [
        "PENDING",
        "ACTIVE",
        "CANCELLATION_REQUESTED",
        "CREATED",
        "DOI_ACCEPTED",
    ] {
        let mut raw = account();
        raw["id"] = json!(state);
        raw["state"] = json!(state);
        accounts.push(raw);
    }
    let mut junior = account();
    junior["configuration"] = json!("JUNIOR");
    accounts.push(junior);
    let projected = project_list(&list(accounts)).unwrap();
    let items = projected["accounts"].as_array().unwrap();
    assert_eq!(
        items
            .iter()
            .map(|a| a["state"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["ACTIVE", "CANCELLATION_REQUESTED", "PENDING"]
    );
    assert_eq!(
        items[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "currency",
            "deposit_amount",
            "display_name",
            "fixed_term_account_id",
            "fixed_term_months",
            "maturity_date",
            "nominal_interest_rate",
            "owner_kind",
            "state"
        ]
    );
}

#[test]
fn fixed_term_direct_ids_cannot_bypass_owner_or_onboarding_filters() {
    for (configuration, state) in [
        ("JUNIOR", "PENDING"),
        ("JUNIOR", "ACTIVE"),
        ("JUNIOR", "CANCELLATION_REQUESTED"),
        ("JUNIOR", "CREATED"),
        ("JUNIOR", "DOI_ACCEPTED"),
        ("SINGLE", "CREATED"),
        ("SINGLE", "DOI_ACCEPTED"),
    ] {
        let mut raw = account();
        raw["configuration"] = json!(configuration);
        raw["state"] = json!(state);
        assert_eq!(
            project_list(&list(vec![raw.clone()])).unwrap(),
            json!({"accounts": []})
        );
        assert_eq!(
            project_details(&details(raw), "ft-1")
                .unwrap_err()
                .to_string(),
            NOT_FOUND
        );
    }
    for response in [json!({"account": null}), details(Value::Null)] {
        let error = project_details(&response, "unknown").unwrap_err();
        assert_eq!(error.to_string(), NOT_FOUND);
        assert_eq!(classify_error(&error).code, "fixed_term_account_not_found");
        assert_eq!(classify_error(&error).exit_code, 10);
    }
    let mismatch = project_details(&details(account()), "different-id").unwrap_err();
    assert_eq!(
        classify_error(&mismatch).code,
        "fixed_term_response_invalid"
    );
    assert_eq!(classify_error(&mismatch).exit_code, 30);
}

#[test]
fn fixed_term_exclusions_do_not_hide_unknown_ownership_or_states() {
    for (configuration, state, invalid_field) in [
        ("UNKNOWN", "CREATED", "configuration"),
        ("UNKNOWN", "DOI_ACCEPTED", "configuration"),
        ("JUNIOR", "FUTURE_STATE", "state"),
    ] {
        let mut raw = account();
        raw["configuration"] = json!(configuration);
        raw["state"] = json!(state);
        for result in [
            project_list(&list(vec![account(), raw.clone()])),
            project_details(&details(raw), "ft-1"),
        ] {
            let error = result.unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Fixed-Term response invalid: invalid or missing {invalid_field}")
            );
            assert_eq!(classify_error(&error).code, "fixed_term_response_invalid");
            assert_eq!(classify_error(&error).exit_code, 30);
        }
    }
}

#[test]
fn fixed_term_unavailable_contractual_fields_remain_null_in_all_supported_states() {
    for state in ["PENDING", "ACTIVE", "CANCELLATION_REQUESTED"] {
        let mut raw = account();
        raw["state"] = json!(state);
        for field in [
            "fixedTerm",
            "endDate",
            "startDate",
            "effectiveInterestRate",
            "currentAccruedAmount",
            "estimatedPayoutAmount",
            "trusteeBalanceBreakdown",
            "personalizations",
        ] {
            raw[field] = Value::Null;
        }
        raw["autoRenewal"] = json!(true);
        let output = project_details(&details(raw.clone()), "ft-1").unwrap();
        for field in [
            "fixed_term_months",
            "maturity_date",
            "contractual_start_date",
            "effective_interest_rate",
            "current_accrued_gross_interest",
            "estimated_payout_amount",
            "allocation",
        ] {
            assert_eq!(output[field], Value::Null, "{field}");
        }
        assert_eq!(output["display_name"], "Fixed Term");
        assert_eq!(output["state"], state);
        assert_eq!(output["auto_renewal_enabled"], true);
        raw.as_object_mut().unwrap().remove("estimatedPayoutAmount");
        assert_eq!(
            project_details(&details(raw), "ft-1").unwrap()["estimated_payout_amount"],
            Value::Null
        );
        assert!(render(&output).contains(&"estimated_payout_amount: <unavailable>".into()));
    }
    assert_eq!(
        render(&json!({"accounts": []})),
        ["No supported Fixed-Term accounts found."]
    );
}

#[test]
fn fixed_term_optional_display_name_falls_back_without_hiding_the_account() {
    for personalizations in [
        None,
        Some(Value::Null),
        Some(json!({})),
        Some(json!({"name": null})),
        Some(json!({"name": "  "})),
        Some(json!("malformed")),
        Some(json!({"name": 42})),
    ] {
        let mut raw = account();
        if let Some(value) = personalizations {
            raw["personalizations"] = value;
        } else {
            raw.as_object_mut().unwrap().remove("personalizations");
        }
        let listed = project_list(&list(vec![raw.clone()])).unwrap();
        let detailed = project_details(&details(raw), "ft-1").unwrap();
        for projected in [&listed["accounts"][0], &detailed] {
            assert_eq!(projected["display_name"], "Fixed Term");
            assert_eq!(projected["fixed_term_account_id"], "ft-1");
            assert_eq!(projected["deposit_amount"], "10000.00");
        }
    }
}

#[test]
fn fixed_term_months_reject_non_integer_json_values_without_coercion() {
    for term in [json!("12"), json!(12.0), json!(true), json!([]), json!({})] {
        let mut raw = account();
        raw["fixedTerm"] = term;
        for result in [
            project_list(&list(vec![raw.clone()])),
            project_details(&details(raw), "ft-1"),
        ] {
            let error = result.unwrap_err();
            assert_eq!(
                error.to_string(),
                "Fixed-Term response invalid: invalid or missing fixedTerm"
            );
            assert_eq!(classify_error(&error).code, "fixed_term_response_invalid");
            assert_eq!(classify_error(&error).exit_code, 30);
        }
    }
}

#[test]
fn fixed_term_invalid_backend_values_fail_closed() {
    let assert_invalid_response = |result: anyhow::Result<Value>, context: &str| {
        let error = result.expect_err(context);
        let classified = classify_error(&error);
        assert_eq!(classified.code, "fixed_term_response_invalid", "{context}");
        assert_eq!(classified.exit_code, 30, "{context}");
    };
    for (field, invalid) in [
        ("id", json!(" ")),
        ("state", json!("FUTURE_STATE")),
        ("configuration", json!("UNKNOWN")),
        ("totalAmount", json!(true)),
        ("interestRate", json!("NaN")),
        ("fixedTerm", json!(1.5)),
        ("fixedTerm", json!(0)),
        ("fixedTerm", json!(-1)),
        ("endDate", json!({"time": "2027-09-01"})),
    ] {
        let mut raw = account();
        raw[field] = invalid;
        for result in [
            project_list(&list(vec![raw.clone()])),
            project_details(&details(raw), "ft-1"),
        ] {
            assert_invalid_response(result, field);
        }
    }
    for (field, invalid) in [
        ("autoRenewal", json!("false")),
        ("startDate", json!({"time": "tomorrow"})),
        ("trusteeBalanceBreakdown", json!({"balances": "bad"})),
        (
            "trusteeBalanceBreakdown",
            json!({"balances": [], "snapshotDateTime": "bad"}),
        ),
    ] {
        let mut raw = account();
        raw[field] = invalid;
        assert_invalid_response(project_details(&details(raw), "ft-1"), field);
    }
    for response in [
        json!({}),
        json!({"account": null}),
        json!({"account": {}}),
        json!({"account": {"fixedTermSavingsAccounts": null}}),
    ] {
        assert_invalid_response(project_list(&response), "list response envelope");
    }
    for (raw, expected) in [
        (json!(12.5), "12.5"),
        (json!("+1.2300e-2"), "+1.2300e-2"),
        (json!(".5"), ".5"),
        (
            json!("10230.12345678901234567890"),
            "10230.12345678901234567890",
        ),
    ] {
        let mut value = account();
        value["totalAmount"] = raw;
        let listed = project_list(&list(vec![value.clone()])).unwrap();
        let detailed = project_details(&details(value), "ft-1").unwrap();
        for projected in [&listed["accounts"][0], &detailed] {
            assert_eq!(projected["deposit_amount"], expected);
        }
    }
    for invalid in ["", "+", "1.2.3", "1e", "1e2e3", "Infinity", "0x12", "1,000"] {
        let mut raw = account();
        raw["totalAmount"] = json!(invalid);
        for result in [
            project_list(&list(vec![raw.clone()])),
            project_details(&details(raw), "ft-1"),
        ] {
            assert_invalid_response(result, invalid);
        }
        let mut payout = account();
        payout["estimatedPayoutAmount"] = json!(invalid);
        assert_invalid_response(project_details(&details(payout), "ft-1"), invalid);
    }
}

#[test]
fn fixed_term_queries_are_named_reads_with_only_approved_fields() {
    use graphql_parser::query::{Definition, OperationDefinition, Selection, SelectionSet};
    fn fields<'a>(selection: &SelectionSet<'a, &'a str>, output: &mut Vec<String>) {
        for item in &selection.items {
            let Selection::Field(field) = item else {
                panic!("unexpected fragment")
            };
            output.push(field.name.to_string());
            fields(&field.selection_set, output);
        }
    }
    for (query, name) in [
        (LIST_QUERY, "ListFixedTermAccounts"),
        (DETAILS_QUERY, "GetFixedTermAccount"),
    ] {
        let document = graphql_parser::parse_query::<&str>(query).unwrap();
        assert_eq!(document.definitions.len(), 1);
        let Definition::Operation(OperationDefinition::Query(operation)) = &document.definitions[0]
        else {
            panic!("expected a query")
        };
        assert_eq!(operation.name, Some(name));
        let mut selected = Vec::new();
        fields(&operation.selection_set, &mut selected);
        let mut expected = vec![
            "account",
            "id",
            "state",
            "fixedTerm",
            "interestRate",
            "totalAmount",
            "configuration",
            "endDate",
            "time",
            "personalizations",
            "name",
        ];
        if name == "ListFixedTermAccounts" {
            expected.push("fixedTermSavingsAccounts");
        } else {
            expected.extend([
                "fixedTermSavingsAccount",
                "effectiveInterestRate",
                "currentAccruedAmount",
                "estimatedPayoutAmount",
                "autoRenewal",
                "startDate",
                "time",
                "trusteeBalanceBreakdown",
                "balances",
                "trusteeName",
                "balance",
                "snapshotDateTime",
            ]);
        }
        selected.sort();
        expected.sort();
        assert_eq!(selected, expected);
    }
}

struct ConfigDirGuard(Option<std::ffi::OsString>);

impl Drop for ConfigDirGuard {
    fn drop(&mut self) {
        // All environment-mutating unit tests hold the repository's test-env lock.
        unsafe {
            match &self.0 {
                Some(value) => std::env::set_var("SC_CONFIG_DIR", value),
                None => std::env::remove_var("SC_CONFIG_DIR"),
            }
        }
    }
}

#[test]
fn fixed_term_authenticated_dispatch_uses_scoped_queries_and_existing_error_handling() {
    let _lock = crate::lock_test_env();
    let tmp = tempfile::tempdir().unwrap();
    let _config_dir = ConfigDirGuard(std::env::var_os("SC_CONFIG_DIR"));
    unsafe {
        std::env::set_var("SC_CONFIG_DIR", tmp.path());
    }
    let mut server = Server::new();
    let _channel = crate::channel::TestEnvConfigOverrideGuard::set(EnvConfig {
        graphql_url: server.url(),
        auth: AuthConfig {
            issuer: "https://issuer.test".into(),
            audience: "https://audience.test".into(),
            client_id: "client-id".into(),
        },
    });
    let config = AppConfig {
        auth: RuntimeAuthConfig {
            session_backend: SessionBackendPreference::File,
            signing_key_backend: DpopKeyBackend::File,
            pkcs11: None,
        },
        trade_controls: None,
    };
    let key = crate::dpop::DpopKeyMaterial::load_or_create_for_options(
        &crate::channel::current_dpop_runtime_options(&config),
    )
    .unwrap();
    let mut sessions = SessionManager::new(&config).unwrap();
    sessions
        .save_active(&StoredSession {
            env: crate::channel::current_env(),
            mode: Some(SessionMode::LocalReadOnly),
            dpop_jwk_thumbprint: Some(key.jwk_thumbprint().unwrap()),
            session: Session {
                access_token: "test-token".into(),
                refresh_token: None,
                id_token: None,
                expires_at: Some(9_999_999_999),
                person_id: "person-1".into(),
                source: LoginSource::DeviceCode,
            },
        })
        .unwrap();
    for (args, query, operation, variables, response, expected) in [
        (
            vec!["list"],
            LIST_QUERY,
            "ListFixedTermAccounts",
            json!({"accountId": "person-1"}),
            list(vec![account()]),
            None,
        ),
        (
            vec!["details", "--fixed-term-account-id", "ft-1"],
            DETAILS_QUERY,
            "GetFixedTermAccount",
            json!({"accountId": "person-1", "fixedTermAccountId": "ft-1"}),
            details(account()),
            None,
        ),
        (
            vec!["details", "--fixed-term-account-id", "unknown"],
            DETAILS_QUERY,
            "GetFixedTermAccount",
            json!({"accountId": "person-1", "fixedTermAccountId": "unknown"}),
            details(Value::Null),
            Some("fixed_term_account_not_found"),
        ),
        (
            vec!["list"],
            LIST_QUERY,
            "ListFixedTermAccounts",
            json!({"accountId": "person-1"}),
            list(vec![]),
            None,
        ),
    ] {
        let request = server
            .mock("POST", "/")
            .match_header("authorization", "DPoP test-token")
            .match_header("dpop", Matcher::Regex("^.+\\..+\\..+$".into()))
            .match_body(Matcher::Json(
                json!({"query": query, "operation_name": operation, "variables": variables}),
            ))
            .with_header("content-type", "application/json")
            .with_body(json!({"data": response}).to_string())
            .create();
        let mut cli_args = vec!["sc", "fixed-term"];
        cli_args.extend(args);
        cli_args.push("--json");
        let command = Cli::parse_from(cli_args).command;
        assert!(
            crate::execution_context::ExecutionContext::from_command(&command)
                .requests_machine_output()
        );
        let result = crate::run_machine_command(command, &config, &mut sessions);
        if let Some(code) = expected {
            assert_eq!(classify_error(&result.unwrap_err()).code, code);
        } else {
            let expected = if operation == "ListFixedTermAccounts" {
                project_list(&response)
            } else {
                project_details(&response, "ft-1")
            };
            assert_eq!(result.unwrap(), expected.unwrap());
        }
        request.assert();
    }
    let request = server
        .mock("POST", "/")
        .with_header("content-type", "application/json")
        .with_body(
            json!({"data": list(vec![]), "errors": [{"message": "backend unavailable"}]})
                .to_string(),
        )
        .create();
    let command = Cli::parse_from(["sc", "fixed-term", "list", "--json"]).command;
    assert!(crate::run_machine_command(command, &config, &mut sessions).is_err());
    request.assert();
}
