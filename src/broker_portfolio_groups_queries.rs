use anyhow::Result;
use serde_json::{Value, json};

use crate::broker_queries::BrokerInput;

pub(crate) const BROKER_PORTFOLIO_GROUPS_QUERY: &str = r#"
query BrokerPortfolioGroups($accountId: ID!, $portfolioId: ID!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      inventory {
        portfolioGroups {
          offerAllowsAdditionalPortfolioGroup
          maxPortfolioGroupsPerPortfolioReached
          items {
            id
            details {
              id
              name
              description
            }
            numberOfPendingOrders
            savingsPlansAmount
            performance {
              id
              valuation
              currency
              performancesByTimeframe {
                timeframe
                performance
                simpleAbsoluteReturn
              }
            }
            items {
              id
              isin
              name
              type
            }
          }
        }
        ungroupedInventoryItems {
          items {
            id
            isin
            name
            type
          }
        }
      }
    }
  }
}
"#;

pub(crate) const BROKER_CREATE_PORTFOLIO_GROUP_MUTATION: &str = r#"
mutation BrokerCreatePortfolioGroup($portfolioId: ID!, $input: CreatePortfolioGroupInput!) {
  createPortfolioGroup(portfolioId: $portfolioId, input: $input) {
    portfolioGroup {
      id
    }
  }
}
"#;

pub(crate) const BROKER_UPDATE_PORTFOLIO_GROUP_MUTATION: &str = r#"
mutation BrokerUpdatePortfolioGroup($portfolioId: ID!, $input: ModifyPortfolioGroupInput!) {
  modifyPortfolioGroup(portfolioId: $portfolioId, input: $input) {
    id
  }
}
"#;

pub(crate) const BROKER_DELETE_PORTFOLIO_GROUP_MUTATION: &str = r#"
mutation BrokerDeletePortfolioGroup($portfolioId: ID!, $input: DeletePortfolioGroupInput!) {
  deletePortfolioGroup(portfolioId: $portfolioId, input: $input) {
    id
  }
}
"#;

pub(crate) const BROKER_ASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION: &str = r#"
mutation BrokerAssignPortfolioGroupItems($portfolioId: ID!, $input: ModifyPortfolioGroupInput!) {
  modifyPortfolioGroup(portfolioId: $portfolioId, input: $input) {
    id
  }
}
"#;

pub(crate) const BROKER_UNASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION: &str = r#"
mutation BrokerUnassignPortfolioGroupItems($portfolioId: ID!, $input: ModifyPortfolioGroupInput!) {
  modifyPortfolioGroup(portfolioId: $portfolioId, input: $input) {
    id
  }
}
"#;

pub(crate) fn broker_portfolio_groups_variables(input: &BrokerInput) -> Result<Value> {
    Ok(json!({
        "accountId": input.account_id_value(),
        "portfolioId": input.portfolio_id_value(),
    }))
}

pub(crate) fn broker_create_portfolio_group_variables(
    portfolio_id: &str,
    name: &str,
    description: Option<&str>,
) -> Value {
    json!({
        "portfolioId": portfolio_id,
        "input": {
            "name": name,
            "description": description,
        },
    })
}

pub(crate) fn broker_update_portfolio_group_variables(
    portfolio_id: &str,
    group_id: &str,
    name: &str,
    description: Option<&str>,
) -> Value {
    json!({
        "portfolioId": portfolio_id,
        "input": {
            "id": group_id,
            "portfolioGroupDetails": {
                "name": name,
                "description": description,
            },
        },
    })
}

pub(crate) fn broker_delete_portfolio_group_variables(portfolio_id: &str, group_id: &str) -> Value {
    json!({
        "portfolioId": portfolio_id,
        "input": {
            "id": group_id,
        },
    })
}

pub(crate) fn broker_assign_portfolio_group_items_variables(
    portfolio_id: &str,
    group_id: &str,
    isins: &[String],
) -> Value {
    broker_modify_portfolio_group_items_variables(portfolio_id, group_id, isins, &[])
}

pub(crate) fn broker_unassign_portfolio_group_items_variables(
    portfolio_id: &str,
    group_id: &str,
    isins: &[String],
) -> Value {
    broker_modify_portfolio_group_items_variables(portfolio_id, group_id, &[], isins)
}

fn broker_modify_portfolio_group_items_variables(
    portfolio_id: &str,
    group_id: &str,
    items_to_add: &[String],
    items_to_remove: &[String],
) -> Value {
    json!({
        "portfolioId": portfolio_id,
        "input": {
            "id": group_id,
            "portfolioGroupItems": {
                "itemsToAdd": items_to_add,
                "itemsToRemove": items_to_remove,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use graphql_parser::query::parse_query;
    use serde_json::json;

    use super::{
        BROKER_ASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION, BROKER_CREATE_PORTFOLIO_GROUP_MUTATION,
        BROKER_DELETE_PORTFOLIO_GROUP_MUTATION, BROKER_PORTFOLIO_GROUPS_QUERY,
        BROKER_UNASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION, BROKER_UPDATE_PORTFOLIO_GROUP_MUTATION,
        broker_assign_portfolio_group_items_variables, broker_create_portfolio_group_variables,
        broker_delete_portfolio_group_variables, broker_portfolio_groups_variables,
        broker_unassign_portfolio_group_items_variables, broker_update_portfolio_group_variables,
    };
    use crate::broker_queries::BrokerInput;

    #[test]
    fn broker_portfolio_groups_variables_map_input() {
        let input = BrokerInput::new("acc-1", "port-1", false).expect("input");

        let variables = broker_portfolio_groups_variables(&input).expect("variables");

        assert_eq!(
            variables,
            json!({
                "accountId": "acc-1",
                "portfolioId": "port-1",
            })
        );
    }

    #[test]
    fn broker_portfolio_groups_query_parses_as_graphql_document() {
        parse_query::<String>(BROKER_PORTFOLIO_GROUPS_QUERY)
            .expect("query should parse as valid GraphQL");
    }

    #[test]
    fn lifecycle_mutation_documents_parse_as_graphql_documents() {
        for document in [
            BROKER_CREATE_PORTFOLIO_GROUP_MUTATION,
            BROKER_UPDATE_PORTFOLIO_GROUP_MUTATION,
            BROKER_DELETE_PORTFOLIO_GROUP_MUTATION,
            BROKER_ASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION,
            BROKER_UNASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION,
        ] {
            parse_query::<String>(document).expect("mutation should parse as valid GraphQL");
        }
    }

    #[test]
    fn lifecycle_mutation_variables_preserve_expected_backend_shapes() {
        assert_eq!(
            broker_create_portfolio_group_variables("port-1", "AI portfolio", Some("tracked")),
            json!({
                "portfolioId": "port-1",
                "input": {"name": "AI portfolio", "description": "tracked"},
            })
        );
        assert_eq!(
            broker_update_portfolio_group_variables("port-1", "group-1", "Renamed", None),
            json!({
                "portfolioId": "port-1",
                "input": {
                    "id": "group-1",
                    "portfolioGroupDetails": {"name": "Renamed", "description": null},
                },
            })
        );
        assert_eq!(
            broker_delete_portfolio_group_variables("port-1", "group-1"),
            json!({"portfolioId": "port-1", "input": {"id": "group-1"}})
        );
        let isins = vec!["US0378331005".to_string(), "IE00B4ND3602".to_string()];
        assert_eq!(
            broker_assign_portfolio_group_items_variables("port-1", "group-1", &isins),
            json!({
                "portfolioId": "port-1",
                "input": {"id": "group-1", "portfolioGroupItems": {
                    "itemsToAdd": ["US0378331005", "IE00B4ND3602"],
                    "itemsToRemove": [],
                }},
            })
        );
        assert_eq!(
            broker_unassign_portfolio_group_items_variables("port-1", "group-1", &isins),
            json!({
                "portfolioId": "port-1",
                "input": {"id": "group-1", "portfolioGroupItems": {
                    "itemsToAdd": [],
                    "itemsToRemove": ["US0378331005", "IE00B4ND3602"],
                }},
            })
        );
    }
}
