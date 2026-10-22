use super::*;
use graphql_parser::query::parse_query;

#[test]
fn overnight_discovery_variables_map_input() {
    let vars = overnight_discovery_variables("person-1").expect("vars");

    assert_eq!(vars["accountId"], "person-1");
}

#[test]
fn overnight_summary_variables_map_input() {
    let input = OvernightSummaryInput::new("person-1", "sav-1").expect("input");
    let vars = overnight_summary_variables(&input).expect("vars");

    assert_eq!(vars["accountId"], "person-1");
    assert_eq!(vars["savingsAccountId"], "sav-1");
}

#[test]
fn overnight_summary_input_rejects_blank_ids() {
    let err = OvernightSummaryInput::new(" ", "sav-1").expect_err("input should fail");
    assert!(err.to_string().contains("account_id"));

    let err = OvernightSummaryInput::new("person-1", " ").expect_err("input should fail");
    assert!(err.to_string().contains("savings_account_id"));
}

#[test]
fn overnight_queries_parse_as_graphql() {
    for query in [DISCOVER_OVERNIGHT_ACCOUNTS_QUERY, OVERNIGHT_SUMMARY_QUERY] {
        parse_query::<String>(query).expect("query should parse as valid GraphQL");
    }
}

#[test]
fn overnight_summary_query_requests_interest_rate() {
    assert!(OVERNIGHT_SUMMARY_QUERY.contains("depositInterestRate"));
}

#[test]
fn overnight_transactions_variables_normalize_filters_and_dates() {
    let options = OvernightTransactionsOptions::new(
        20,
        Some("cursor-1"),
        &[
            "cash transfer in".to_string(),
            "interest".to_string(),
            "future cash event".to_string(),
        ],
        Some("interest"),
        Some("2026-03-01T00:00:00Z"),
        Some("2026-03-02T00:00:00Z"),
    )
    .expect("options");
    let input = OvernightTransactionsInput::new("person-1", "sav-1", options).expect("input");

    let vars = overnight_transactions_variables(&input);

    assert_eq!(vars["accountId"], "person-1");
    assert_eq!(vars["savingsAccountId"], "sav-1");
    assert_eq!(vars["input"]["pageSize"], 20);
    assert_eq!(vars["input"]["cursor"], "cursor-1");
    assert_eq!(
        vars["input"]["type"],
        serde_json::json!(["CASH_TRANSFER_IN", "FUTURE_CASH_EVENT", "INTEREST"])
    );
    assert_eq!(vars["input"]["fromTime"], 1_772_323_200_i64);
    assert_eq!(vars["input"]["toTime"], 1_772_409_600_i64);
    assert!(vars["input"].get("status").is_none());
    assert!(vars["input"].get("isin").is_none());
}

#[test]
fn overnight_transactions_options_reject_invalid_filter_and_time_range() {
    let err =
        OvernightTransactionsOptions::new(20, None, &["cash@event".to_string()], None, None, None)
            .expect_err("invalid filter characters should fail");
    assert!(err.to_string().contains("type_filter"));

    let err = OvernightTransactionsOptions::new(
        20,
        None,
        &[],
        None,
        Some("2026-03-02T00:00:00Z"),
        Some("2026-03-01T00:00:00Z"),
    )
    .expect_err("inverted range should fail");
    assert!(err.to_string().contains("from_time"));
}

#[test]
fn overnight_transactions_options_validate_page_and_timestamp_inputs() {
    for page_size in [0, 101] {
        let err = OvernightTransactionsOptions::new(page_size, None, &[], None, None, None)
            .expect_err("out-of-range page size should fail");
        assert!(err.to_string().contains("page_size"));
    }

    for timestamp in ["", "2026-03-01"] {
        let err = OvernightTransactionsOptions::new(20, None, &[], None, Some(timestamp), None)
            .expect_err("invalid timestamp should fail");
        assert!(err.to_string().contains("from_time"));
    }
}

#[test]
fn overnight_transactions_options_normalize_and_deduplicate_optional_filters() {
    let options = OvernightTransactionsOptions::new(
        20,
        Some("  "),
        &["interest".to_string(), "INTEREST".to_string()],
        Some("  "),
        None,
        None,
    )
    .expect("options");
    let input = OvernightTransactionsInput::new(" person-1 ", " sav-1 ", options).expect("input");

    let vars = overnight_transactions_variables(&input);

    assert_eq!(vars["accountId"], "person-1");
    assert_eq!(vars["savingsAccountId"], "sav-1");
    assert_eq!(vars["input"]["cursor"], serde_json::Value::Null);
    assert_eq!(vars["input"]["type"], serde_json::json!(["INTEREST"]));
    assert_eq!(vars["input"]["searchTerm"], serde_json::Value::Null);
    assert!(vars["input"].get("status").is_none());
    assert!(vars["input"].get("isin").is_none());
}

#[test]
fn overnight_transactions_options_normalize_camel_case_and_repeated_separators() {
    let options = OvernightTransactionsOptions::new(
        20,
        None,
        &[
            "CashTransfer-In".to_string(),
            "cash--transfer__out".to_string(),
        ],
        None,
        None,
        None,
    )
    .expect("options");
    let input = OvernightTransactionsInput::new("person-1", "sav-1", options).expect("input");

    let vars = overnight_transactions_variables(&input);

    assert_eq!(
        vars["input"]["type"],
        serde_json::json!(["CASH_TRANSFER_IN", "CASH_TRANSFER_OUT"])
    );
}

#[test]
fn overnight_transactions_query_requests_cash_transaction_fields() {
    parse_query::<String>(OVERNIGHT_TRANSACTIONS_QUERY).expect("valid GraphQL");
    for field in [
        "moreTransactions",
        "cashTransactionType",
        "relatedIsin",
        "custodian",
    ] {
        assert!(OVERNIGHT_TRANSACTIONS_QUERY.contains(field));
    }
}
