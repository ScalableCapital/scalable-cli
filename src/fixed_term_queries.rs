pub(crate) const LIST_QUERY: &str = r#"
query ListFixedTermAccounts($accountId: ID!) {
  account(id: $accountId) {
    fixedTermSavingsAccounts {
      id state fixedTerm interestRate totalAmount configuration
      endDate { time }
      personalizations { name }
    }
  }
}
"#;

pub(crate) const DETAILS_QUERY: &str = r#"
query GetFixedTermAccount($accountId: ID!, $fixedTermAccountId: ID!) {
  account(id: $accountId) {
    fixedTermSavingsAccount(id: $fixedTermAccountId) {
      id state fixedTerm interestRate effectiveInterestRate totalAmount
      currentAccruedAmount estimatedPayoutAmount configuration autoRenewal
      startDate { time }
      endDate { time }
      personalizations { name }
      trusteeBalanceBreakdown {
        balances { trusteeName balance }
        snapshotDateTime
      }
    }
  }
}
"#;
