use anyhow::{Result, anyhow};
use serde_json::{Value, json};

pub const DISCOVER_OVERNIGHT_ACCOUNTS_QUERY: &str = r#"
query DiscoverOvernightAccounts($accountId: ID!) {
  account(id: $accountId) {
    savingsAccounts {
      __typename
      id
      owners {
        firstName
        lastName
      }
      personalizations {
        name
      }
      state
    }
  }
  productList(personId: $accountId) {
    minors {
      id
      name
      type
      onboardingState
      state
      owners {
        id
        firstName
        lastName
      }
      productOffering {
        name
        displayName
      }
    }
  }
}
"#;

pub const OVERNIGHT_SUMMARY_QUERY: &str = r#"
query OvernightSummary($accountId: ID!, $savingsAccountId: ID!) {
  account(id: $accountId) {
    savingsAccount(id: $savingsAccountId) {
      id
      ... on OvernightSavingsAccount {
        interests {
          currentAccruedAmount
          currentInterestBearingAmount
          depositAccruedLifetimeAmount
          depositInterestRate
          estimatedNextPayoutAmount
          nextPayoutDate {
            epochSecond
          }
        }
        nextPayoutDate {
          epochSecond
        }
        totalAmount
      }
    }
  }
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OvernightSummaryInput {
    account_id: String,
    savings_account_id: String,
}

impl OvernightSummaryInput {
    pub(crate) fn new(account_id: &str, savings_account_id: &str) -> Result<Self> {
        Ok(Self {
            account_id: required_non_empty(account_id, "account_id")?,
            savings_account_id: required_non_empty(savings_account_id, "savings_account_id")?,
        })
    }

    pub(crate) fn account_id(&self) -> &str {
        &self.account_id
    }

    pub(crate) fn savings_account_id(&self) -> &str {
        &self.savings_account_id
    }
}

pub(crate) fn overnight_discovery_variables(account_id: &str) -> Result<Value> {
    Ok(json!({
        "accountId": required_non_empty(account_id, "account_id")?,
    }))
}

pub(crate) fn overnight_summary_variables(input: &OvernightSummaryInput) -> Result<Value> {
    Ok(json!({
        "accountId": required_non_empty(input.account_id(), "account_id")?,
        "savingsAccountId": required_non_empty(input.savings_account_id(), "savings_account_id")?,
    }))
}

fn required_non_empty(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Overnight input invalid: field '{field}' must be a non-empty string"
        ));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
#[path = "overnight_queries_tests.rs"]
mod tests;
