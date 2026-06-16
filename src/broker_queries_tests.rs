use super::*;
use graphql_parser::query::parse_query;
use serde_json::json;

fn broker_input(include_year_to_date: bool, quote_source: Option<&str>) -> BrokerInput {
    BrokerInput::new("acc-1", "port-1", include_year_to_date, quote_source).expect("input")
}

fn sample_derivatives_search_args() -> crate::cli::BrokerDerivativesSearchArgs {
    crate::cli::BrokerDerivativesSearchArgs {
        portfolio_id: Some("port-1".to_string()),
        underlying: "US0378331005".to_string(),
        derivative_type: crate::cli::BrokerDerivativeType::Knockout,
        limit: 25,
        offset: 50,
        issuer: vec![crate::cli::BrokerDerivativeIssuer::Hsbc],
        strategy: crate::cli::BrokerDerivativeStrategy::Long,
        product_subcategory: vec![crate::cli::BrokerDerivativeKnockoutSubcategory::Turbo],
        leverage_min: Some("2".to_string()),
        leverage_max: Some("5".to_string()),
        knockout_barrier_min: Some("180".to_string()),
        knockout_barrier_max: Some("200".to_string()),
        strike_min: Some("175".to_string()),
        strike_max: Some("195".to_string()),
        omega_min: None,
        omega_max: None,
        delta_min: None,
        delta_max: None,
        factor_min: None,
        factor_max: None,
        expiry_from: None,
        expiry_to: None,
        sort_field: Some(crate::cli::BrokerDerivativeSortField::Leverage),
        sort_order: Some(crate::cli::BrokerDerivativeSortOrder::Desc),
        json: true,
    }
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
fn broker_derivatives_search_variables_map_knockout_input() {
    let mut args = sample_derivatives_search_args();
    args.expiry_from = Some("2026-06-01".to_string());
    args.expiry_to = Some("2026-12-31".to_string());

    let normalized = normalize_broker_derivatives_search_query_input(&args).expect("normalized");
    let vars =
        broker_derivatives_search_variables(&broker_input(false, None), &normalized).expect("vars");
    assert_eq!(vars["accountId"], "acc-1");
    assert_eq!(vars["portfolioId"], "port-1");
    assert_eq!(
        vars["input"]["knockoutInput"]["underlyingIsin"],
        "US0378331005"
    );
    assert_eq!(vars["input"]["knockoutInput"]["pagination"]["limit"], 25);
    assert_eq!(vars["input"]["knockoutInput"]["pagination"]["offset"], 50);
    assert_eq!(vars["input"]["knockoutInput"]["strategy"], "LONG");
    assert_eq!(vars["input"]["knockoutInput"]["issuers"][0], "HSBC");
    assert_eq!(
        vars["input"]["knockoutInput"]["productSubcategories"][0],
        "TURBO"
    );
    assert_eq!(vars["input"]["knockoutInput"]["leverageRange"]["min"], "2");
    assert_eq!(vars["input"]["knockoutInput"]["leverageRange"]["max"], "5");
    assert_eq!(
        vars["input"]["knockoutInput"]["expiryDate"]["startDate"],
        "2026-06-01"
    );
    assert_eq!(
        vars["input"]["knockoutInput"]["expiryDate"]["endDate"],
        "2026-12-31"
    );
    assert_eq!(
        vars["input"]["knockoutInput"]["expiryDate"]["isOpenEnd"],
        false
    );
    assert_eq!(
        vars["input"]["knockoutInput"]["knockoutBarrier"]["min"],
        "180"
    );
    assert_eq!(vars["input"]["knockoutInput"]["strike"]["max"], "195");
    assert_eq!(
        vars["input"]["knockoutInput"]["sortBy"]["field"],
        "LEVERAGE"
    );
    assert_eq!(vars["input"]["knockoutInput"]["sortBy"]["order"], "DESC");
    assert_eq!(vars["input"]["warrantInput"], json!(null));
    assert_eq!(vars["input"]["factorCertificateInput"], json!(null));
}

#[test]
fn broker_derivatives_search_variables_reject_invalid_strategy_for_type() {
    let mut args = sample_derivatives_search_args();
    args.derivative_type = crate::cli::BrokerDerivativeType::Warrant;
    args.strategy = crate::cli::BrokerDerivativeStrategy::Long;

    let err = normalize_broker_derivatives_search_query_input(&args).unwrap_err();
    assert!(err.to_string().contains("not supported"));
}

#[test]
fn broker_derivatives_search_variables_reject_knockout_fields_for_warrant_type() {
    let mut args = sample_derivatives_search_args();
    args.derivative_type = crate::cli::BrokerDerivativeType::Warrant;
    args.strategy = crate::cli::BrokerDerivativeStrategy::Call;

    let err = normalize_broker_derivatives_search_query_input(&args).unwrap_err();
    assert!(err.to_string().contains("product_subcategory"));
}

#[test]
fn broker_derivatives_search_variables_reject_offset_above_graphql_int_max() {
    let mut args = sample_derivatives_search_args();
    args.offset = (i32::MAX as u32) + 1;

    let err = normalize_broker_derivatives_search_query_input(&args).unwrap_err();
    assert!(err.to_string().contains("field 'offset'"));
}

#[test]
fn broker_derivatives_search_variables_reject_invalid_warrant_expiry_range() {
    let mut args = sample_derivatives_search_args();
    args.derivative_type = crate::cli::BrokerDerivativeType::Warrant;
    args.strategy = crate::cli::BrokerDerivativeStrategy::Call;
    args.product_subcategory.clear();
    args.leverage_min = None;
    args.leverage_max = None;
    args.knockout_barrier_min = None;
    args.knockout_barrier_max = None;
    args.strike_min = None;
    args.strike_max = None;
    args.expiry_from = Some("2026-12-31".to_string());
    args.expiry_to = Some("2026-06-01".to_string());
    args.sort_field = Some(crate::cli::BrokerDerivativeSortField::ExpiryDate);

    let err = normalize_broker_derivatives_search_query_input(&args).unwrap_err();
    assert!(err.to_string().contains("expiry_from"));
}

#[test]
fn broker_derivatives_search_variables_map_warrant_input() {
    let mut args = sample_derivatives_search_args();
    args.derivative_type = crate::cli::BrokerDerivativeType::Warrant;
    args.strategy = crate::cli::BrokerDerivativeStrategy::Call;
    args.product_subcategory.clear();
    args.leverage_min = None;
    args.leverage_max = None;
    args.knockout_barrier_min = None;
    args.knockout_barrier_max = None;
    args.omega_min = Some("3".to_string());
    args.omega_max = Some("9".to_string());
    args.delta_min = Some("-0.6".to_string());
    args.delta_max = Some("-0.2".to_string());
    args.expiry_from = Some("2026-06-01".to_string());
    args.expiry_to = Some("2026-12-31".to_string());
    args.sort_field = Some(crate::cli::BrokerDerivativeSortField::Delta);

    let normalized = normalize_broker_derivatives_search_query_input(&args).expect("normalized");
    let vars =
        broker_derivatives_search_variables(&broker_input(false, None), &normalized).expect("vars");

    assert_eq!(
        vars["input"]["warrantInput"]["underlyingIsin"],
        "US0378331005"
    );
    assert_eq!(vars["input"]["warrantInput"]["strategy"], "CALL");
    assert_eq!(vars["input"]["warrantInput"]["omegaRange"]["min"], "3");
    assert_eq!(vars["input"]["warrantInput"]["deltaRange"]["min"], "-0.6");
    assert_eq!(
        vars["input"]["warrantInput"]["expiryDate"]["startDate"],
        "2026-06-01"
    );
    assert_eq!(vars["input"]["warrantInput"]["sortBy"]["field"], "DELTA");
    assert_eq!(vars["input"]["knockoutInput"], json!(null));
    assert_eq!(vars["input"]["factorCertificateInput"], json!(null));
}

#[test]
fn broker_derivatives_search_variables_map_factor_input() {
    let mut args = sample_derivatives_search_args();
    args.derivative_type = crate::cli::BrokerDerivativeType::Factor;
    args.strategy = crate::cli::BrokerDerivativeStrategy::Short;
    args.product_subcategory.clear();
    args.leverage_min = None;
    args.leverage_max = None;
    args.knockout_barrier_min = None;
    args.knockout_barrier_max = None;
    args.strike_min = None;
    args.strike_max = None;
    args.factor_min = Some("2".to_string());
    args.factor_max = Some("8".to_string());
    args.sort_field = Some(crate::cli::BrokerDerivativeSortField::Factor);

    let normalized = normalize_broker_derivatives_search_query_input(&args).expect("normalized");
    let vars =
        broker_derivatives_search_variables(&broker_input(false, None), &normalized).expect("vars");

    assert_eq!(
        vars["input"]["factorCertificateInput"]["underlyingIsin"],
        "US0378331005"
    );
    assert_eq!(vars["input"]["factorCertificateInput"]["strategy"], "SHORT");
    assert_eq!(
        vars["input"]["factorCertificateInput"]["factorRange"]["min"],
        "2"
    );
    assert_eq!(
        vars["input"]["factorCertificateInput"]["sortBy"]["field"],
        "FACTOR"
    );
    assert_eq!(vars["input"]["knockoutInput"], json!(null));
    assert_eq!(vars["input"]["warrantInput"], json!(null));
}

#[test]
fn broker_derivatives_search_variables_reject_missing_sort_pair() {
    let mut args = sample_derivatives_search_args();
    args.sort_order = None;

    let err = normalize_broker_derivatives_search_query_input(&args).unwrap_err();
    assert!(err.to_string().contains("sort_order"));
}

#[test]
fn broker_derivatives_search_variables_reject_factor_invalid_sort_field() {
    let mut args = sample_derivatives_search_args();
    args.derivative_type = crate::cli::BrokerDerivativeType::Factor;
    args.strategy = crate::cli::BrokerDerivativeStrategy::Short;
    args.product_subcategory.clear();
    args.leverage_min = None;
    args.leverage_max = None;
    args.knockout_barrier_min = None;
    args.knockout_barrier_max = None;
    args.strike_min = None;
    args.strike_max = None;
    args.factor_min = Some("2".to_string());
    args.factor_max = Some("8".to_string());
    args.sort_field = Some(crate::cli::BrokerDerivativeSortField::Leverage);

    let err = normalize_broker_derivatives_search_query_input(&args).unwrap_err();
    assert!(err.to_string().contains("sort field"));
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
fn broker_create_or_update_savings_plan_variables_accept_plus_prefixed_decimals() {
    let vars = broker_create_or_update_savings_plan_variables(
        "port-1",
        "US0378331005",
        "+100",
        "MONTHLY",
        5,
        "2026-04",
        "+1.5",
        "REFERENCE_ACCOUNT",
        None,
        None,
    )
    .expect("vars");
    assert_eq!(vars["input"]["amount"], "+100");
    assert_eq!(vars["input"]["configuration"]["dynamizationRate"], "+1.5");
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
        BROKER_DERIVATIVES_SEARCH_QUERY,
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

#[test]
fn broker_derivatives_search_query_uses_non_conflicting_warrant_expiry_shape() {
    let warrant_fragment = BROKER_DERIVATIVES_SEARCH_QUERY
        .split("... on WarrantSearchResult {")
        .nth(1)
        .and_then(|section| {
            section
                .split("... on FactorCertificateSearchResult {")
                .next()
        })
        .expect("warrant fragment");

    assert!(warrant_fragment.contains("expiryDate {\n              epochDay\n            }"));
    assert!(!warrant_fragment.contains("expiryDate {\n              date"));
}
