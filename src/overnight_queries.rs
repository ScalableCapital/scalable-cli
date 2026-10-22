use anyhow::{Result, anyhow};
use chrono::{DateTime, FixedOffset};
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

pub const OVERNIGHT_TRANSACTIONS_QUERY: &str = r#"
query OvernightTransactions(
  $accountId: ID!
  $savingsAccountId: ID!
  $input: SavingsAccountCashTransactionInput!
) {
  account(id: $accountId) {
    savingsAccount(id: $savingsAccountId) {
      id
      moreTransactions(input: $input) {
        cursor
        total
        transactions {
          id
          currency
          type
          status
          isCancellation
          lastEventDateTime
          description
          cashTransactionType
          amount
          custodian
          relatedIsin
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OvernightTransactionsOptions {
    page_size: u16,
    cursor: Option<String>,
    type_filter: Option<Vec<String>>,
    search_term: Option<String>,
    from_time_seconds: Option<i64>,
    to_time_seconds: Option<i64>,
}

impl OvernightTransactionsOptions {
    pub(crate) fn new(
        page_size: u16,
        cursor: Option<&str>,
        type_filter: &[String],
        search_term: Option<&str>,
        from_time: Option<&str>,
        to_time: Option<&str>,
    ) -> Result<Self> {
        if page_size == 0 || page_size > 100 {
            return Err(anyhow!(
                "Overnight input invalid: field 'page_size' must be between 1 and 100"
            ));
        }

        let type_filter = normalize_enum_filter_values(type_filter, "type_filter")?;

        let from_time = from_time
            .map(|value| parse_iso_8601_timestamp(value, "from_time"))
            .transpose()?;
        let to_time = to_time
            .map(|value| parse_iso_8601_timestamp(value, "to_time"))
            .transpose()?;
        if let (Some(from), Some(to)) = (&from_time, &to_time)
            && from > to
        {
            return Err(anyhow!(
                "Overnight input invalid: field 'from_time' must be before or equal to 'to_time'"
            ));
        }

        Ok(Self {
            page_size,
            cursor: normalize_optional_non_empty_string(cursor),
            type_filter,
            search_term: normalize_optional_non_empty_string(search_term),
            from_time_seconds: from_time.map(|value| value.timestamp()),
            to_time_seconds: to_time.map(|value| value.timestamp()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OvernightTransactionsInput {
    account_id: String,
    savings_account_id: String,
    options: OvernightTransactionsOptions,
}

impl OvernightTransactionsInput {
    pub(crate) fn new(
        account_id: &str,
        savings_account_id: &str,
        options: OvernightTransactionsOptions,
    ) -> Result<Self> {
        Ok(Self {
            account_id: required_non_empty(account_id, "account_id")?,
            savings_account_id: required_non_empty(savings_account_id, "savings_account_id")?,
            options,
        })
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

pub(crate) fn overnight_transactions_variables(input: &OvernightTransactionsInput) -> Value {
    json!({
        "accountId": input.account_id,
        "savingsAccountId": input.savings_account_id,
        "input": {
            "pageSize": input.options.page_size,
            "cursor": input.options.cursor,
            "type": input.options.type_filter,
            "searchTerm": input.options.search_term,
            "fromTime": input.options.from_time_seconds,
            "toTime": input.options.to_time_seconds,
        }
    })
}

pub(crate) fn overnight_transactions_type_filter_help() -> String {
    "Backend transaction type filter (repeatable). Input is normalized to uppercase snake case. For example: `interest`, `deposit`, `withdrawal`, `cash transfer in`, `cash transfer out`.".to_string()
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

fn normalize_optional_non_empty_string(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn parse_iso_8601_timestamp(raw: &str, field: &str) -> Result<DateTime<FixedOffset>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Overnight input invalid: field '{field}' must be a non-empty ISO-8601 timestamp"
        ));
    }
    DateTime::parse_from_rfc3339(trimmed).map_err(|_| {
        anyhow!(
            "Overnight input invalid: field '{field}' must be an ISO-8601 timestamp with timezone"
        )
    })
}

fn normalize_enum_filter_values(values: &[String], field: &str) -> Result<Option<Vec<String>>> {
    if values.is_empty() {
        return Ok(None);
    }

    let mut normalized = values
        .iter()
        .map(|value| normalize_enum_filter_value(value, field))
        .collect::<Result<Vec<_>>>()?;
    normalized.sort();
    normalized.dedup();

    Ok((!normalized.is_empty()).then_some(normalized))
}

fn normalize_enum_filter_value(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Overnight input invalid: field '{field}' contains an empty value"
        ));
    }

    let mut normalizer = EnumFilterNormalizer::new(trimmed.len());

    for character in trimmed.chars() {
        match classify_filter_character(character, field)? {
            FilterCharacter::Word(character) => normalizer.push_word_character(character),
            FilterCharacter::Separator => normalizer.push_separator(),
        }
    }

    let normalized = normalizer.finish();
    if normalized.is_empty() {
        return Err(anyhow!(
            "Overnight input invalid: field '{field}' contains an empty value"
        ));
    }
    Ok(normalized)
}

enum FilterCharacter {
    Word(char),
    Separator,
}

fn classify_filter_character(character: char, field: &str) -> Result<FilterCharacter> {
    if character.is_ascii_alphanumeric() {
        return Ok(FilterCharacter::Word(character));
    }
    if matches!(character, '_' | '-' | ' ') {
        return Ok(FilterCharacter::Separator);
    }
    Err(anyhow!(
        "Overnight input invalid: field '{field}' contains unsupported characters"
    ))
}

struct EnumFilterNormalizer {
    value: String,
    previous_was_word: bool,
    previous_was_lower_or_digit: bool,
}

impl EnumFilterNormalizer {
    fn new(input_len: usize) -> Self {
        Self {
            value: String::with_capacity(input_len + 4),
            previous_was_word: false,
            previous_was_lower_or_digit: false,
        }
    }

    fn push_word_character(&mut self, character: char) {
        if character.is_ascii_uppercase()
            && self.previous_was_lower_or_digit
            && !self.value.ends_with('_')
        {
            self.value.push('_');
        }
        self.value.push(character.to_ascii_uppercase());
        self.previous_was_word = true;
        self.previous_was_lower_or_digit =
            character.is_ascii_lowercase() || character.is_ascii_digit();
    }

    fn push_separator(&mut self) {
        if self.previous_was_word && !self.value.ends_with('_') {
            self.value.push('_');
        }
        self.previous_was_word = false;
        self.previous_was_lower_or_digit = false;
    }

    fn finish(self) -> String {
        self.value.trim_matches('_').to_string()
    }
}

#[cfg(test)]
#[path = "overnight_queries_tests.rs"]
mod tests;
