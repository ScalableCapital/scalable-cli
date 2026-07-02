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
