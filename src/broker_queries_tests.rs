use super::*;
use graphql_parser::query::parse_query;

fn broker_input(include_year_to_date: bool, quote_source: Option<&str>) -> BrokerInput {
    BrokerInput::new("acc-1", "port-1", include_year_to_date, quote_source).expect("input")
}

#[test]
fn broker_overview_variables_map_input() {
    let vars = broker_overview_variables(&broker_input(true, None)).expect("vars");
    assert_eq!(vars["accountId"], "acc-1");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["includeYearToDate"], true);
}

#[test]
fn broker_analytics_variables_map_input() {
    let vars = broker_analytics_variables(&broker_input(false, None)).expect("vars");
    assert_eq!(vars["accountId"], "acc-1");
    assert_eq!(vars["portfolioId"], "port-1");
}

#[test]
fn broker_transactions_variables_map_input() {
    let normalized = normalize_broker_transactions_query_input(
        50,
        Some("cursor-1"),
        &["buy".to_string(), "reinvestmentDistribution".to_string()],
        &["filled".to_string(), "settled".to_string()],
        Some("apple"),
        Some("2026-03-01T00:00:00Z"),
        Some("2026-03-10T23:59:59Z"),
        Some("US0378331005"),
        true,
    )
    .expect("normalized");
    let vars =
        broker_transactions_variables_from_normalized(&broker_input(false, None), &normalized);
    assert_eq!(vars["accountId"], "acc-1");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["input"]["pageSize"], 50);
    assert_eq!(vars["input"]["cursor"], "cursor-1");
    assert_eq!(vars["input"]["type"][0], "BUY");
    assert_eq!(vars["input"]["type"][1], "REINVESTMENT_DISTRIBUTION");
    assert_eq!(vars["input"]["status"][0], "FILLED");
    assert_eq!(vars["input"]["status"][1], "SETTLED");
    assert_eq!(vars["input"]["searchTerm"], "apple");
    assert_eq!(vars["input"]["isin"], "US0378331005");
    assert_eq!(vars["input"]["fromTime"], 1772323200_i64);
    assert_eq!(vars["input"]["toTime"], 1773187199_i64);
    assert_eq!(vars["input"]["includeReinvestmentSubtypes"], true);
}

#[test]
fn broker_transactions_variables_reject_invalid_range() {
    let err = normalize_broker_transactions_query_input(
        20,
        None,
        &[],
        &[],
        None,
        Some("2026-03-11T00:00:00Z"),
        Some("2026-03-10T00:00:00Z"),
        None,
        false,
    )
    .unwrap_err();
    assert!(err.to_string().contains("from_time"));
}

#[test]
fn broker_transactions_variables_reject_invalid_fractional_range_within_same_second() {
    let err = normalize_broker_transactions_query_input(
        20,
        None,
        &[],
        &[],
        None,
        Some("2026-01-01T00:00:00.900Z"),
        Some("2026-01-01T00:00:00.100Z"),
        None,
        false,
    )
    .unwrap_err();
    assert!(err.to_string().contains("from_time"));
}

#[test]
fn broker_transactions_variables_reject_page_size_above_max() {
    let err = normalize_broker_transactions_query_input(
        101,
        None,
        &[],
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .unwrap_err();
    assert!(err.to_string().contains("page_size"));
}

#[test]
fn broker_transactions_variables_reject_unsupported_type_filter() {
    let err = normalize_broker_transactions_query_input(
        20,
        None,
        &["CASH_TRANSACTION".to_string()],
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("type_filter"));
    assert!(message.contains("CASH_TRANSACTION"));
    assert!(message.contains("not supported"));
    assert!(message.contains("BUY"));
}

#[test]
fn broker_transactions_variables_reject_unsupported_status_filter() {
    let err = normalize_broker_transactions_query_input(
        20,
        None,
        &[],
        &["DONE".to_string()],
        None,
        None,
        None,
        None,
        false,
    )
    .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("status"));
    assert!(message.contains("DONE"));
    assert!(message.contains("not supported"));
    assert!(message.contains("FILLED"));
}

#[test]
fn broker_transaction_details_variables_map_input() {
    let vars =
        broker_transaction_details_variables(&broker_input(false, None), " tx-1 ").expect("vars");
    assert_eq!(vars["accountId"], "acc-1");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["transactionId"], "tx-1");
}

#[test]
fn broker_transaction_details_variables_reject_blank_transaction_id() {
    let err = broker_transaction_details_variables(&broker_input(false, None), "  ").unwrap_err();
    assert!(err.to_string().contains("transaction_id"));
}

#[test]
fn broker_transaction_details_query_aliases_conflicting_fields() {
    for required in [
        "securityTransactionHistory: transactionHistory",
        "cashTransactionHistory: transactionHistory",
        "nonTradeAveragePrice: averagePrice",
        "nonTradeSecurityAmount: totalAmount",
        "nonTradeSecurityTransactionHistory: transactionHistory",
        "eltifTransactionHistory: transactionHistory",
    ] {
        assert!(
            BROKER_TRANSACTION_DETAILS_QUERY.contains(required),
            "query should contain {required}"
        );
    }
}

#[test]
fn broker_holdings_variables_map_input() {
    let vars = broker_holdings_variables(&broker_input(true, Some("CONSOLIDATED"))).expect("vars");
    assert_eq!(vars["accountId"], "acc-1");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["includeYearToDate"], true);
    assert_eq!(vars["quoteSource"], "CONSOLIDATED");
}

#[test]
fn broker_quote_variables_map_input() {
    let vars = broker_quote_variables(&broker_input(true, Some("CONSOLIDATED")), " US0378331005 ")
        .expect("vars");
    assert_eq!(vars["accountId"], "acc-1");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["includeYearToDate"], true);
    assert_eq!(vars["quoteSource"], "CONSOLIDATED");
    assert_eq!(vars["isin"], "US0378331005");
}

#[test]
fn broker_quote_variables_reject_blank_isin() {
    let err = broker_quote_variables(&broker_input(false, None), "  ").unwrap_err();
    assert!(err.to_string().contains("field 'isin'"));
}

#[test]
fn broker_quote_query_contains_app_parity_identity_and_tick_fields() {
    for required in [
        "security(isin: $isin)",
        "id",
        "bidPrice",
        "askPrice",
        "timestampUtc",
        "performancesByTimeframe",
        "simpleAbsoluteReturn",
    ] {
        assert!(
            BROKER_QUOTE_QUERY.contains(required),
            "quote query should contain {required}"
        );
    }
    assert!(
        !BROKER_QUOTE_QUERY.contains("\n          time\n"),
        "quote query should not request deprecated QuoteTick.time"
    );
}

#[test]
fn broker_add_to_watchlist_variables_map_input() {
    let vars = broker_add_to_watchlist_variables("port-1", " US0378331005 ").expect("vars");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["isin"], "US0378331005");
    assert_eq!(vars["input"]["isin"], "US0378331005");
}

#[test]
fn broker_remove_from_watchlist_variables_map_input() {
    let vars = broker_remove_from_watchlist_variables("port-1", "US0378331005").expect("vars");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["isin"], "US0378331005");
    assert_eq!(vars["input"]["isin"], "US0378331005");
}

#[test]
fn broker_remove_savings_plan_variables_map_input() {
    let vars = broker_remove_savings_plan_variables("port-1", "US0378331005").expect("vars");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["isin"], "US0378331005");
}

#[test]
fn broker_watchlist_mutation_variables_reject_blank_isin() {
    let add_err = broker_add_to_watchlist_variables("port-1", "  ").unwrap_err();
    assert!(add_err.to_string().contains("field 'isin'"));

    let remove_err = broker_remove_from_watchlist_variables("port-1", "").unwrap_err();
    assert!(remove_err.to_string().contains("field 'isin'"));
}

#[test]
fn broker_remove_savings_plan_variables_reject_blank_isin() {
    let err = broker_remove_savings_plan_variables("port-1", "").unwrap_err();
    assert!(err.to_string().contains("field 'isin'"));
}

#[test]
fn broker_savings_plans_variables_map_input() {
    let vars = broker_savings_plans_variables(&broker_input(false, None)).expect("vars");
    assert_eq!(vars["accountId"], "acc-1");
    assert_eq!(vars["portfolioId"], "port-1");
}

#[test]
fn broker_savings_plan_config_variables_map_input() {
    let vars = broker_savings_plan_config_variables(&broker_input(false, None), "US0378331005")
        .expect("vars");
    assert_eq!(vars["accountId"], "acc-1");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["isin"], "US0378331005");
}

#[test]
fn broker_create_or_update_savings_plan_variables_map_input() {
    let vars = broker_create_or_update_savings_plan_variables(
        "port-1",
        "US0378331005",
        "100",
        "MONTHLY",
        5,
        "2026-04",
        "1.5",
        "REFERENCE_ACCOUNT",
        Some("app-1"),
        Some("warn-v1"),
    )
    .expect("vars");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["input"]["isin"], "US0378331005");
    assert_eq!(vars["input"]["amount"], "100");
    assert_eq!(vars["input"]["configuration"]["frequency"], "MONTHLY");
    assert_eq!(vars["input"]["configuration"]["dayOfTheMonth"], 5);
    assert_eq!(vars["input"]["configuration"]["yearMonth"], "2026-04");
    assert_eq!(
        vars["input"]["configuration"]["paymentMethod"],
        "REFERENCE_ACCOUNT"
    );
    assert_eq!(vars["input"]["appropriatenessId"], "app-1");
    assert_eq!(
        vars["input"]["acknowledgedAppropriatenessWarningVersion"],
        "warn-v1"
    );
}

#[test]
fn broker_create_or_update_savings_plan_variables_reject_invalid_year_month() {
    let err = broker_create_or_update_savings_plan_variables(
        "port-1",
        "US0378331005",
        "100",
        "MONTHLY",
        5,
        "2026-13",
        "1.5",
        "REFERENCE_ACCOUNT",
        None,
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("year_month"));
}

#[test]
fn broker_create_or_update_savings_plan_variables_accept_zero_dynamization_rate() {
    let vars = broker_create_or_update_savings_plan_variables(
        "port-1",
        "US0378331005",
        "100",
        "MONTHLY",
        5,
        "2026-04",
        "0",
        "REFERENCE_ACCOUNT",
        None,
        None,
    )
    .expect("vars");
    assert_eq!(vars["input"]["configuration"]["dynamizationRate"], "0");
}

#[test]
fn broker_add_price_alert_variables_map_input() {
    let vars = broker_add_price_alert_variables("port-1", "US0378331005", "123.45").expect("vars");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["isin"], "US0378331005");
    assert_eq!(vars["price"], "123.45");
}

#[test]
fn broker_add_crypto_price_alert_variables_map_input() {
    let vars = broker_add_crypto_price_alert_variables("port-1", "BTC", "123.45").expect("vars");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["ticker"], "BTC");
    assert_eq!(vars["price"], "123.45");
}

#[test]
fn broker_remove_price_alert_variables_map_input() {
    let vars = broker_remove_price_alert_variables("port-1", "alert-1").expect("vars");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(vars["alertId"], "alert-1");
}

#[test]
fn broker_remove_price_alert_variables_reject_blank_alert_id() {
    let err = broker_remove_price_alert_variables("port-1", "  ").unwrap_err();
    assert!(err.to_string().contains("alert_id"));
}

#[test]
fn broker_add_price_alert_variables_reject_invalid_price() {
    let err = broker_add_price_alert_variables("port-1", "US0378331005", "-1").unwrap_err();
    assert!(err.to_string().contains("positive decimal"));
}

#[test]
fn broker_query_documents_parse_as_graphql_documents() {
    let queries = [
        BROKER_OVERVIEW_QUERY,
        BROKER_ANALYTICS_QUERY,
        BROKER_TRANSACTIONS_QUERY,
        BROKER_TRANSACTION_DETAILS_QUERY,
        BROKER_HOLDINGS_QUERY,
        BROKER_WATCHLIST_QUERY,
        BROKER_ADD_TO_WATCHLIST_MUTATION,
        BROKER_REMOVE_FROM_WATCHLIST_MUTATION,
        BROKER_REMOVE_SAVINGS_PLAN_MUTATION,
        BROKER_SEARCH_QUERY,
        BROKER_QUOTE_QUERY,
        BROKER_SECURITY_NEWS_QUERY,
        BROKER_PRICE_ALERTS_QUERY,
        BROKER_CRYPTO_PRICE_ALERTS_QUERY,
        BROKER_ADD_PRICE_ALERT_MUTATION,
        BROKER_ADD_CRYPTO_PRICE_ALERT_MUTATION,
        BROKER_REMOVE_PRICE_ALERT_MUTATION,
        BROKER_REMOVE_CRYPTO_PRICE_ALERT_MUTATION,
        BROKER_LIMITS_QUERY,
        BROKER_SAVINGS_PLANS_QUERY,
        BROKER_SAVINGS_PLAN_CONFIG_QUERY,
        BROKER_CREATE_OR_UPDATE_SAVINGS_PLAN_MUTATION,
        BROKER_SAVINGS_PLAN_BY_ISIN_QUERY,
    ];

    for query in queries {
        parse_query::<String>(query).expect("query should parse as valid GraphQL");
    }
}
