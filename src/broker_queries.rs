use anyhow::{Result, anyhow};
use chrono::{DateTime, FixedOffset};
use serde_json::{Value, json};

const VALID_BROKER_TRANSACTION_TYPE_FILTERS: &[&str] = &[
    "BUY",
    "SELL",
    "SAVINGS_PLAN",
    "DEPOSIT",
    "WITHDRAWAL",
    "DISTRIBUTION",
    "FEE",
    "INTEREST",
    "TAX",
    "TAX_RETURN",
    "SWAP_IN",
    "SWAP_OUT",
    "TRANSFER_IN",
    "TRANSFER_OUT",
    "CURRENCY_SWITCH_BUY",
    "CURRENCY_SWITCH_SELL",
    "CASH_TRANSFER_IN",
    "CASH_TRANSFER_OUT",
    "POCKET_MONEY",
    "REINVESTMENT",
    "REINVESTMENT_DISTRIBUTION",
    "REINVESTMENT_POCKET_MONEY",
];

const VALID_BROKER_TRANSACTION_STATUSES: &[&str] = &[
    "CREATED",
    "REQUESTED",
    "PENDING",
    "PARTIAL_FILLED",
    "FILLED",
    "SETTLED",
    "CANCELLED",
    "CANCEL_REQUESTED",
    "EXPIRED",
    "REJECTED",
    "CONFIRMED",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedBrokerTransactionsQueryInput {
    pub(crate) page_size: u16,
    pub(crate) cursor: Option<String>,
    pub(crate) type_filter: Option<Vec<String>>,
    pub(crate) status: Option<Vec<String>>,
    pub(crate) search_term: Option<String>,
    pub(crate) from_time_seconds: Option<i64>,
    pub(crate) to_time_seconds: Option<i64>,
    pub(crate) isin: Option<String>,
    pub(crate) include_reinvestment_subtypes: bool,
}

pub(crate) fn broker_transactions_type_filter_help() -> String {
    format_enum_filter_help(
        "Transaction type filter (repeatable). Accepted values",
        VALID_BROKER_TRANSACTION_TYPE_FILTERS,
        "Input is normalized, so values like `buy`, `cash transfer in`, and `cash-transfer-in` are accepted.",
    )
}

pub(crate) fn broker_transactions_status_help() -> String {
    format_enum_filter_help(
        "Transaction status filter (repeatable). Accepted values",
        VALID_BROKER_TRANSACTION_STATUSES,
        "Input is normalized, so values like `filled` and `partial filled` are accepted.",
    )
}

pub const BROKER_OVERVIEW_QUERY: &str = r#"
query BrokerOverview(
  $accountId: ID!
  $portfolioId: ID!
  $includeYearToDate: Boolean!
) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      valuation(includeYearToDate: $includeYearToDate) {
        valuation
        securitiesValuation
        cryptoValuation
        timestampUtc {
          time
        }
        lastInventoryUpdateTimestampUtc {
          time
        }
        timeWeightedReturnByTimeframe {
          timeframe
          performance
          simpleAbsoluteReturn
        }
      }
    }
  }
}
"#;

pub const BROKER_ANALYTICS_QUERY: &str = r#"
query BrokerAnalytics($accountId: ID!, $portfolioId: ID!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      portfolioAnalysis {
        type
        ... on InvalidSecuritiesPortfolioAnalysisResult {
          invalidSecurities {
            id
            isin
            name
            type
          }
          portfolioCoverage
        }
        result {
          id
          lastUpdated {
            time
          }
          healthChecks {
            id
            items {
              id
              type
              healthScore
              state
              color {
                sixDigitNotationValue
              }
              numberOfItemsInPortfolio
              maxItems
            }
          }
          scenarios {
            id
            items {
              id
              type
              portfolioPerformance
              benchmarkPerformance
              securities {
                id
                isin
                name
                type
              }
            }
          }
          allocations {
            id
            items {
              id
              type
              positions {
                id
                name
                weight
                valuation
                contributors {
                  id
                  weight
                  underlyingAsset {
                    ... on Security {
                      isin
                      name
                      inventory {
                        position {
                          filled
                        }
                      }
                    }
                    ... on CryptoCoin {
                      ticker
                      name
                    }
                  }
                }
                subpositions {
                  id
                  name
                  weight
                  valuation
                  contributors {
                    id
                    weight
                    underlyingAsset {
                      ... on Security {
                        isin
                        name
                        inventory {
                          position {
                            filled
                          }
                        }
                      }
                      ... on CryptoCoin {
                        ticker
                        name
                      }
                    }
                  }
                }
              }
            }
          }
          equityCompanyStyles {
            id
            marketCaps {
              id
              type
              weight
              items {
                id
                type
                weight
                contributors {
                  id
                  weight
                  underlyingAsset {
                    ... on Security {
                      isin
                      name
                    }
                    ... on CryptoCoin {
                      ticker
                      name
                    }
                  }
                }
              }
              contributors {
                id
                weight
                underlyingAsset {
                  ... on Security {
                    isin
                    name
                  }
                  ... on CryptoCoin {
                    ticker
                    name
                  }
                }
              }
            }
          }
          fixedIncomeRatings {
            id
            investmentGrade {
              id
              name
              weight
              contributors {
                id
                weight
                underlyingAsset {
                  ... on Security {
                    isin
                    name
                    inventory {
                      position {
                        filled
                      }
                    }
                  }
                  ... on CryptoCoin {
                    ticker
                    name
                  }
                }
              }
            }
            speculativeGrade {
              id
              name
              weight
              contributors {
                id
                weight
                underlyingAsset {
                  ... on Security {
                    isin
                    name
                    inventory {
                      position {
                        filled
                      }
                    }
                  }
                  ... on CryptoCoin {
                    ticker
                    name
                  }
                }
              }
            }
            unratedGrade {
              id
              name
              weight
              contributors {
                id
                weight
                underlyingAsset {
                  ... on Security {
                    isin
                    name
                    inventory {
                      position {
                        filled
                      }
                    }
                  }
                  ... on CryptoCoin {
                    ticker
                    name
                  }
                }
              }
            }
            showSpeculativeInvestmentWarning
          }
          payments {
            id
            totalDistributions
            totalInterest
          }
          trialPeriod {
            id
            isEligible
            isRunning
            startDate {
              time
            }
            durationInHours
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_HOLDINGS_QUERY: &str = r#"
query BrokerHoldings(
  $accountId: ID!
  $portfolioId: ID!
  $includeYearToDate: Boolean!
  $quoteSource: MarketDataSource
) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      inventory {
        items {
          isin
          name
          type
          inventory {
            position {
              filled
              pending
              blocked
              fifoPrice
            }
          }
          portfolioIsinPerformance {
            valuation
            currency
          }
          quoteTick(source: $quoteSource, includeYearToDate: $includeYearToDate) {
            midPrice
            currency
            timestampUtc {
              time
            }
            isOutdated
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_TRANSACTIONS_QUERY: &str = r#"
query BrokerTransactions($accountId: ID!, $portfolioId: ID!, $input: BrokerTransactionInput!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      moreTransactions(input: $input) {
        cursor
        total
        transactions {
          __typename
          id
          currency
          type
          status
          isCancellation
          lastEventDateTime
          description
          custodian
          documents {
            id
            url
            label
          }
          ... on BrokerSecurityTransactionSummary {
            isin
            securityTransactionType
            quantity
            amount
            side
            limitPrice
            stopPrice
          }
          ... on BrokerCashTransactionSummary {
            relatedIsin
            cashTransactionType
            amount
          }
          ... on BrokerNonTradeSecurityTransactionSummary {
            isin
            nonTradeSecurityTransactionType
            quantity
            amount
          }
          ... on BrokerEltifTransactionSummary {
            isin
            securityTransactionType
            eltifQuantity
            amount
            side
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_TRANSACTION_DETAILS_QUERY: &str = r#"
query BrokerTransactionDetails($accountId: ID!, $portfolioId: ID!, $transactionId: ID!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      transactionDetails(id: $transactionId) {
        __typename
        id
        currency
        type
        documents {
          id
          url
          label
        }
        lastEventDateTime
        isPending
        isCancellation
        security {
          isin
          name
          type
        }
        transactionReference
        ... on BrokerSecurityTransaction {
          side
          status
          numberOfShares {
            filled
            total
          }
          averagePrice
          totalAmount
          finalisationReason
          limitPrice
          stopPrice
          validUntil
          isCancellationRequested
          tradeTransactionAmounts {
            marketValuation
            taxAmount
            transactionFee
            venueFee
            cryptoSpreadFee
          }
          tradingVenue
          fee
          transactionalFee
          taxes
          aggregatedTransactionTaxes {
            totalTax
            capitalGainsTax
            churchTax
            solidarityTax
            sourceTax
            financialTransactionTax
          }
          securityTransactionHistory: transactionHistory {
            state
            time {
              time
              epochSecond
              epochMillisecond
            }
            numberOfShares {
              filled
              total
            }
            executionPrice
          }
          orderKind
          linkedTransactions {
            id
          }
          trailingStopInfo {
            trailType
            trailOffset
            latestStopPriceTimestamp {
              time
              epochSecond
              epochMillisecond
            }
          }
        }
        ... on BrokerCashTransaction {
          cashTransactionType
          amount
          description
          cashTransactionHistory: transactionHistory {
            state
            time {
              time
              epochSecond
              epochMillisecond
            }
          }
          sddiDetails {
            fee
            grossAmount
          }
          taxDetails {
            grossAmount
            taxAmount
          }
          linkedTransactions {
            id
          }
        }
        ... on BrokerNonTradeSecurityTransaction {
          isin
          nonTradeSecurityTransactionType
          quantity
          nonTradeAveragePrice: averagePrice
          nonTradeSecurityAmount: totalAmount
          description
          nonTradeSecurityTransactionHistory: transactionHistory {
            state
            time {
              time
              epochSecond
              epochMillisecond
            }
          }
          linkedTransactions {
            id
          }
        }
        ... on BrokerEltifTransaction {
          status
          side
          orderKind
          amount
          finalisationReason
          eltifQuantity
          executionPrice
          executionDate
          earliestSellDate
          marketValuation
          cancelableDetails {
            daysLeft
            isCancelable
          }
          isMultipleOrdersCancellation
          isInitialInvestment
          tradingVenue
          eltifTransactionHistory: transactionHistory {
            state
            amount
            eltifQuantity
            executionPrice
            time {
              time
              epochSecond
              epochMillisecond
            }
          }
          linkedTransactions {
            id
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_WATCHLIST_QUERY: &str = r#"
query BrokerWatchlist(
  $accountId: ID!
  $portfolioId: ID!
  $includeYearToDate: Boolean!
  $quoteSource: MarketDataSource
) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      watchlist {
        items {
          isin
          name
          type
          quoteTick(source: $quoteSource, includeYearToDate: $includeYearToDate) {
            midPrice
            currency
            timestampUtc {
              time
            }
            isOutdated
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_ADD_TO_WATCHLIST_MUTATION: &str = r#"
mutation BrokerAddToWatchlist($portfolioId: ID!, $isin: ID!, $input: UpdateWatchlistInput!) {
  addToWatchlist(portfolioId: $portfolioId, input: $input) {
    security(isin: $isin) {
      isin
      isOnWatchlist
    }
  }
}
"#;

pub const BROKER_REMOVE_FROM_WATCHLIST_MUTATION: &str = r#"
mutation BrokerRemoveFromWatchlist($portfolioId: ID!, $isin: ID!, $input: UpdateWatchlistInput!) {
  removeFromWatchlist(portfolioId: $portfolioId, input: $input) {
    security(isin: $isin) {
      isin
      isOnWatchlist
    }
  }
}
"#;

pub const BROKER_REMOVE_SAVINGS_PLAN_MUTATION: &str = r#"
mutation BrokerRemoveSavingsPlan($portfolioId: ID!, $isin: ID!) {
  removeSavingsPlan(portfolioId: $portfolioId, input: { isin: $isin }) {
    id
  }
}
"#;

pub const BROKER_SEARCH_QUERY: &str = r#"
query BrokerSecuritySearch(
  $accountId: ID!
  $portfolioId: ID!
  $searchTerm: String!
  $includeYearToDate: Boolean!
  $quoteSource: MarketDataSource
) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      simpleSecuritySearch(searchTerm: $searchTerm) {
        items {
          isin
          name
          type
          quoteTick(source: $quoteSource, includeYearToDate: $includeYearToDate) {
            midPrice
            currency
            timestampUtc {
              time
            }
            isOutdated
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_QUOTE_QUERY: &str = r#"
query BrokerQuote(
  $accountId: ID!
  $portfolioId: ID!
  $isin: ID!
  $includeYearToDate: Boolean!
  $quoteSource: MarketDataSource
) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      security(isin: $isin) {
        id
        isin
        name
        type
        quoteTick(source: $quoteSource, includeYearToDate: $includeYearToDate) {
          id
          isin
          midPrice
          currency
          bidPrice
          askPrice
          isOutdated
          timestampUtc {
            time
          }
          performanceDate {
            date
          }
          performancesByTimeframe {
            timeframe
            performance
            simpleAbsoluteReturn
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_SECURITY_NEWS_QUERY: &str = r#"
query BrokerSecurityNews($isin: ID!, $locale: String!) {
  securityNews(isin: $isin, locale: $locale) {
    isin
    shortNewsSummary
    longNewsSummary
    lastUpdated {
      time
    }
    sources {
      id
      headline
      sourceName
      publicationTime {
        time
      }
    }
  }
}
"#;

pub const BROKER_PRICE_ALERTS_QUERY: &str = r#"
query BrokerPriceAlerts($accountId: ID!, $portfolioId: ID!, $activeOnly: Boolean!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      priceAlerts(activeOnly: $activeOnly) {
        itemsPerInstrument {
          canAddNew
          items {
            id
            direction
            isActive
            price
            triggeredTimestamp {
              time
            }
            security {
              isin
              name
              type
            }
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_CRYPTO_PRICE_ALERTS_QUERY: &str = r#"
query BrokerCryptoPriceAlerts($accountId: ID!, $portfolioId: ID!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      crypto {
        priceAlerts {
          itemsPerInstrument {
            canAddNew
            items {
              id
              direction
              isActive
              price
              triggeredTimestamp {
                time
              }
              coin {
                ticker
                name
              }
            }
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_ADD_PRICE_ALERT_MUTATION: &str = r#"
mutation BrokerAddPriceAlert($portfolioId: ID!, $isin: ID!, $price: PositiveBigDecimal!) {
  addPriceAlert(portfolioId: $portfolioId, input: {isin: $isin, price: $price}) {
    security(isin: $isin) {
      priceAlerts {
        canAddNew
        items {
          id
          direction
          isActive
          price
          triggeredTimestamp {
            time
          }
          security {
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

pub const BROKER_ADD_CRYPTO_PRICE_ALERT_MUTATION: &str = r#"
mutation BrokerAddCryptoPriceAlert($portfolioId: ID!, $ticker: ID!, $price: PositiveBigDecimal!) {
  addCryptoPriceAlert(portfolioId: $portfolioId, input: {ticker: $ticker, price: $price}) {
    crypto {
      coin(ticker: $ticker) {
        ticker
        name
        priceAlerts {
          canAddNew
          items {
            id
            direction
            isActive
            price
            triggeredTimestamp {
              time
            }
            coin {
              ticker
              name
            }
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_REMOVE_PRICE_ALERT_MUTATION: &str = r#"
mutation BrokerRemovePriceAlert($portfolioId: ID!, $alertId: ID!) {
  removePriceAlert(portfolioId: $portfolioId, input: {id: $alertId}) {
    id
  }
}
"#;

pub const BROKER_REMOVE_CRYPTO_PRICE_ALERT_MUTATION: &str = r#"
mutation BrokerRemoveCryptoPriceAlert($portfolioId: ID!, $alertId: ID!) {
  removeCryptoPriceAlert(portfolioId: $portfolioId, input: {id: $alertId}) {
    id
  }
}
"#;

pub const BROKER_LIMITS_QUERY: &str = r#"
query BrokerLimits($accountId: ID!, $portfolioId: ID!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      depositLimits {
        min
        max
      }
      withdrawalLimits {
        min
        max
        maxExcludingCredit
      }
      payments {
        buyingPower {
          cashBalance
          liveLimit
          loaned
          pendingBuyOrdersAmount
          pendingWithdrawalsAmount
          pendingSavingsPlanAmount
          pendingDividendsReinvestmentAmount
          pendingPocketMoneyAmount
          estimatedTaxes
          directDebit
          cashAvailableToInvest
          cashAvailableToInvestWithoutCredit
        }
        derivativesBuyingPower {
          cashAvailableToInvest
          derivativesDirectDebit
          pendingELTIFAmount
          cashAvailableForDerivatives
        }
        withdrawalPower {
          cashAvailableToInvest
          sellTradesAmount
          withdrawalDirectDebit
          cashAvailableForWithdrawal
        }
      }
    }
  }
}
"#;

pub const BROKER_SAVINGS_PLANS_QUERY: &str = r#"
query BrokerSavingsPlans($accountId: ID!, $portfolioId: ID!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      totalSavingsPlanAmount
      inventory {
        items {
          isin
          name
          type
          inventory {
            savingsPlan {
              isin
              amount
              frequency
              dayOfTheMonth
              dynamizationRate
              paymentMethod
              nextExecutionDate {
                date
                epochDay
              }
            }
          }
        }
      }
      crypto {
        coins {
          ticker
          name
          savingsPlanAmount
        }
      }
    }
  }
}
"#;

pub const BROKER_SAVINGS_PLAN_CONFIG_QUERY: &str = r#"
query BrokerSavingsPlanConfig($accountId: ID!, $portfolioId: ID!, $isin: ID!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      security(isin: $isin) {
        isin
        savingsPlanConfiguration {
          schedules {
            dayOfTheMonth
            isEarliest
            isDefault
            yearMonths {
              yearMonth
              isAvailable
            }
          }
          minSavingsPlanAmount
          maxSavingsPlanAmount
          defaultMinSavingsPlanAmount
          dynamizationRates
          defaultDynamizationRate
          paymentMethods
          frequencies
          nextInstructedExecutionDate {
            date
            epochDay
          }
        }
      }
    }
  }
}
"#;

pub const BROKER_CREATE_OR_UPDATE_SAVINGS_PLAN_MUTATION: &str = r#"
mutation BrokerCreateOrUpdateSavingsPlan($portfolioId: ID!, $input: CreateOrUpdateSavingsPlanInput!) {
  createOrUpdateSavingsPlan(portfolioId: $portfolioId, input: $input) {
    id
  }
}
"#;

pub const BROKER_SAVINGS_PLAN_BY_ISIN_QUERY: &str = r#"
query BrokerSavingsPlanByIsin($accountId: ID!, $portfolioId: ID!, $isin: ID!) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      security(isin: $isin) {
        isin
        name
        type
        inventory {
          savingsPlan {
            isin
            amount
            frequency
            dayOfTheMonth
            dynamizationRate
            paymentMethod
            nextExecutionDate {
              date
              epochDay
            }
          }
        }
      }
    }
  }
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerInput {
    account_id: String,
    portfolio_id: String,
    include_year_to_date: bool,
    quote_source: Option<String>,
}

impl BrokerInput {
    pub fn new(
        account_id: &str,
        portfolio_id: &str,
        include_year_to_date: bool,
        quote_source: Option<&str>,
    ) -> Result<Self> {
        let account_id = account_id.trim();
        let portfolio_id = portfolio_id.trim();
        if account_id.is_empty() {
            return Err(anyhow!(
                "Broker input invalid: field 'account_id' must be a non-empty string"
            ));
        }
        if portfolio_id.is_empty() {
            return Err(anyhow!(
                "Broker input invalid: field 'portfolio_id' must be a non-empty string"
            ));
        }

        let quote_source = quote_source
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string);

        Ok(Self {
            account_id: account_id.to_string(),
            portfolio_id: portfolio_id.to_string(),
            include_year_to_date,
            quote_source,
        })
    }

    pub(crate) fn account_id_value(&self) -> Value {
        Value::String(self.account_id.clone())
    }

    pub(crate) fn portfolio_id_value(&self) -> Value {
        Value::String(self.portfolio_id.clone())
    }
}

pub(crate) fn timestamp_value_or_null(raw: Option<&Value>) -> Value {
    match raw {
        Some(value) => value.get("time").cloned().unwrap_or_else(|| value.clone()),
        None => Value::Null,
    }
}

fn normalize_positive_decimal(raw: &str) -> Result<String> {
    normalize_positive_decimal_with_field(raw, "price")
}

fn normalize_positive_decimal_with_field(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must be a positive decimal"
        ));
    }
    let dot_count = trimmed.chars().filter(|c| *c == '.').count();
    let has_only_decimal_chars = trimmed.chars().all(|c| c.is_ascii_digit() || c == '.');
    if !has_only_decimal_chars || dot_count > 1 {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must be a positive decimal"
        ));
    }
    let parsed = trimmed
        .parse::<f64>()
        .map_err(|_| anyhow!("Broker input invalid: field '{field}' must be a positive decimal"))?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must be a positive decimal"
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_non_negative_decimal_with_field(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must be a non-negative decimal"
        ));
    }
    let dot_count = trimmed.chars().filter(|c| *c == '.').count();
    let has_only_decimal_chars = trimmed.chars().all(|c| c.is_ascii_digit() || c == '.');
    if !has_only_decimal_chars || dot_count > 1 {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must be a non-negative decimal"
        ));
    }
    let parsed = trimmed.parse::<f64>().map_err(|_| {
        anyhow!("Broker input invalid: field '{field}' must be a non-negative decimal")
    })?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must be a non-negative decimal"
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_year_month(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.len() != 7 || trimmed.as_bytes().get(4) != Some(&b'-') {
        return Err(anyhow!(
            "Broker input invalid: field 'year_month' must use YYYY-MM format"
        ));
    }
    let year = &trimmed[0..4];
    let month = &trimmed[5..7];
    let valid_year = year.chars().all(|c| c.is_ascii_digit());
    let valid_month = matches!(
        month,
        "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
    );
    if !valid_year || !valid_month {
        return Err(anyhow!(
            "Broker input invalid: field 'year_month' must use YYYY-MM format"
        ));
    }
    Ok(trimmed.to_string())
}

pub fn broker_overview_variables(input: &BrokerInput) -> Result<Value> {
    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
        "includeYearToDate": input.include_year_to_date,
    }))
}

pub fn broker_analytics_variables(input: &BrokerInput) -> Result<Value> {
    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
    }))
}

pub fn broker_holdings_variables(input: &BrokerInput) -> Result<Value> {
    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
        "includeYearToDate": input.include_year_to_date,
        "quoteSource": input.quote_source,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn normalize_broker_transactions_query_input(
    page_size: u16,
    cursor: Option<&str>,
    type_filter: &[String],
    status: &[String],
    search_term: Option<&str>,
    from_time: Option<&str>,
    to_time: Option<&str>,
    isin: Option<&str>,
    include_reinvestment_subtypes: bool,
) -> Result<NormalizedBrokerTransactionsQueryInput> {
    if page_size == 0 || page_size > 100 {
        return Err(anyhow!(
            "Broker input invalid: field 'page_size' must be between 1 and 100"
        ));
    }

    let normalized_cursor = normalize_optional_non_empty_string(cursor);
    let normalized_type_filter = normalize_enum_filter_values(type_filter, "type_filter")?;
    validate_enum_filter_values(
        normalized_type_filter.as_deref(),
        "type_filter",
        VALID_BROKER_TRANSACTION_TYPE_FILTERS,
    )?;
    let normalized_status = normalize_enum_filter_values(status, "status")?;
    validate_enum_filter_values(
        normalized_status.as_deref(),
        "status",
        VALID_BROKER_TRANSACTION_STATUSES,
    )?;
    let normalized_search_term = normalize_optional_non_empty_string(search_term);
    let normalized_isin = normalize_optional_non_empty_string(isin);

    let from_time_parsed = from_time
        .map(|value| parse_iso_8601_timestamp(value, "from_time"))
        .transpose()?;
    let to_time_parsed = to_time
        .map(|value| parse_iso_8601_timestamp(value, "to_time"))
        .transpose()?;
    if let (Some(from), Some(to)) = (&from_time_parsed, &to_time_parsed)
        && from > to
    {
        return Err(anyhow!(
            "Broker input invalid: field 'from_time' must be before or equal to 'to_time'"
        ));
    }
    let from_time_seconds = from_time_parsed.map(epoch_seconds);
    let to_time_seconds = to_time_parsed.map(epoch_seconds);

    Ok(NormalizedBrokerTransactionsQueryInput {
        page_size,
        cursor: normalized_cursor,
        type_filter: normalized_type_filter,
        status: normalized_status,
        search_term: normalized_search_term,
        from_time_seconds,
        to_time_seconds,
        isin: normalized_isin,
        include_reinvestment_subtypes,
    })
}

pub(crate) fn broker_transactions_variables_from_normalized(
    input: &BrokerInput,
    normalized: &NormalizedBrokerTransactionsQueryInput,
) -> Value {
    let query_input = json!({
        "pageSize": normalized.page_size,
        "cursor": normalized.cursor.clone(),
        "type": normalized.type_filter.clone(),
        "status": normalized.status.clone(),
        "searchTerm": normalized.search_term.clone(),
        "fromTime": normalized.from_time_seconds,
        "toTime": normalized.to_time_seconds,
        "isin": normalized.isin.clone(),
        "includeReinvestmentSubtypes": normalized.include_reinvestment_subtypes,
    });

    json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
        "input": query_input,
    })
}

pub fn broker_transaction_details_variables(
    input: &BrokerInput,
    transaction_id: &str,
) -> Result<Value> {
    let transaction_id = transaction_id.trim();
    if transaction_id.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'transaction_id' must be a non-empty string"
        ));
    }

    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
        "transactionId": transaction_id,
    }))
}

pub fn broker_watchlist_variables(input: &BrokerInput) -> Result<Value> {
    broker_holdings_variables(input)
}

pub fn broker_add_to_watchlist_variables(portfolio_id: &str, isin: &str) -> Result<Value> {
    broker_watchlist_mutation_variables(portfolio_id, isin)
}

pub fn broker_remove_from_watchlist_variables(portfolio_id: &str, isin: &str) -> Result<Value> {
    broker_watchlist_mutation_variables(portfolio_id, isin)
}

pub fn broker_remove_savings_plan_variables(portfolio_id: &str, isin: &str) -> Result<Value> {
    let portfolio_id = portfolio_id.trim();
    if portfolio_id.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'portfolio_id' must be a non-empty string"
        ));
    }
    let isin = isin.trim();
    if isin.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'isin' must be a non-empty string"
        ));
    }

    Ok(json!({
        "portfolioId": portfolio_id,
        "isin": isin,
    }))
}

pub fn broker_search_variables(input: &BrokerInput, search_term: &str) -> Result<Value> {
    let mut vars = broker_holdings_variables(input)?
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("Broker input invalid: variables must be an object"))?;
    vars.insert(
        "searchTerm".to_string(),
        Value::String(search_term.to_string()),
    );
    Ok(Value::Object(vars))
}

pub fn broker_quote_variables(input: &BrokerInput, isin: &str) -> Result<Value> {
    let isin = isin.trim();
    if isin.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'isin' must be a non-empty string"
        ));
    }

    let mut vars = broker_holdings_variables(input)?
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("Broker input invalid: variables must be an object"))?;
    vars.insert("isin".to_string(), Value::String(isin.to_string()));
    Ok(Value::Object(vars))
}

pub fn broker_price_alerts_variables(input: &BrokerInput, active_only: bool) -> Result<Value> {
    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
        "activeOnly": active_only,
    }))
}

pub fn broker_crypto_price_alerts_variables(input: &BrokerInput) -> Result<Value> {
    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
    }))
}

pub fn broker_limits_variables(input: &BrokerInput) -> Result<Value> {
    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
    }))
}

pub fn broker_savings_plans_variables(input: &BrokerInput) -> Result<Value> {
    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
    }))
}

pub fn broker_savings_plan_config_variables(input: &BrokerInput, isin: &str) -> Result<Value> {
    let isin = isin.trim();
    if isin.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'isin' must be a non-empty string"
        ));
    }
    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
        "isin": isin,
    }))
}

pub fn broker_savings_plan_by_isin_variables(input: &BrokerInput, isin: &str) -> Result<Value> {
    broker_savings_plan_config_variables(input, isin)
}

#[allow(clippy::too_many_arguments)]
pub fn broker_create_or_update_savings_plan_variables(
    portfolio_id: &str,
    isin: &str,
    amount: &str,
    frequency: &str,
    day_of_month: u8,
    year_month: &str,
    dynamization_rate: &str,
    payment_method: &str,
    appropriateness_id: Option<&str>,
    acknowledged_appropriateness_warning_version: Option<&str>,
) -> Result<Value> {
    let portfolio_id = portfolio_id.trim();
    if portfolio_id.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'portfolio_id' must be a non-empty string"
        ));
    }
    let isin = isin.trim();
    if isin.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'isin' must be a non-empty string"
        ));
    }
    if day_of_month == 0 || day_of_month > 31 {
        return Err(anyhow!(
            "Broker input invalid: field 'day_of_month' must be between 1 and 31"
        ));
    }
    let frequency = frequency.trim();
    if frequency.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'frequency' must be a non-empty string"
        ));
    }
    let payment_method = payment_method.trim();
    if payment_method.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'payment_method' must be a non-empty string"
        ));
    }

    Ok(json!({
        "portfolioId": portfolio_id,
        "input": {
            "isin": isin,
            "amount": normalize_positive_decimal_with_field(amount, "amount")?,
            "configuration": {
                "frequency": frequency,
                "dayOfTheMonth": day_of_month,
                "yearMonth": normalize_year_month(year_month)?,
                "dynamizationRate": normalize_non_negative_decimal_with_field(dynamization_rate, "dynamization_rate")?,
                "paymentMethod": payment_method,
            },
            "appropriatenessId": appropriateness_id
                .map(str::trim)
                .filter(|v| !v.is_empty()),
            "acknowledgedAppropriatenessWarningVersion": acknowledged_appropriateness_warning_version
                .map(str::trim)
                .filter(|v| !v.is_empty()),
        }
    }))
}

pub fn broker_add_price_alert_variables(
    portfolio_id: &str,
    isin: &str,
    price: &str,
) -> Result<Value> {
    Ok(json!({
        "portfolioId": portfolio_id.trim(),
        "isin": isin.trim(),
        "price": normalize_positive_decimal(price)?,
    }))
}

pub fn broker_add_crypto_price_alert_variables(
    portfolio_id: &str,
    ticker: &str,
    price: &str,
) -> Result<Value> {
    Ok(json!({
        "portfolioId": portfolio_id.trim(),
        "ticker": ticker.trim(),
        "price": normalize_positive_decimal(price)?,
    }))
}

pub fn broker_remove_price_alert_variables(portfolio_id: &str, alert_id: &str) -> Result<Value> {
    let portfolio_id = portfolio_id.trim();
    if portfolio_id.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'portfolio_id' must be a non-empty string"
        ));
    }

    let alert_id = alert_id.trim();
    if alert_id.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'alert_id' must be a non-empty string"
        ));
    }

    Ok(json!({
        "portfolioId": portfolio_id,
        "alertId": alert_id,
    }))
}

fn broker_watchlist_mutation_variables(portfolio_id: &str, isin: &str) -> Result<Value> {
    let portfolio_id = portfolio_id.trim();
    if portfolio_id.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'portfolio_id' must be a non-empty string"
        ));
    }

    let isin = isin.trim();
    if isin.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'isin' must be a non-empty string"
        ));
    }

    Ok(json!({
        "portfolioId": portfolio_id,
        "isin": isin,
        "input": {
            "isin": isin,
        }
    }))
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
            "Broker input invalid: field '{field}' must be a non-empty ISO-8601 timestamp"
        ));
    }
    let parsed = DateTime::parse_from_rfc3339(trimmed).map_err(|_| {
        anyhow!("Broker input invalid: field '{field}' must be an ISO-8601 timestamp with timezone")
    })?;
    Ok(parsed)
}

fn epoch_seconds(timestamp: DateTime<FixedOffset>) -> i64 {
    timestamp.timestamp()
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

    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

fn normalize_enum_filter_value(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' contains an empty value"
        ));
    }

    let mut normalized = String::with_capacity(trimmed.len() + 4);
    let mut prev_was_word = false;
    let mut prev_was_lower_or_digit = false;

    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && prev_was_lower_or_digit && !normalized.ends_with('_') {
                normalized.push('_');
            }
            normalized.push(ch.to_ascii_uppercase());
            prev_was_word = true;
            prev_was_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
            continue;
        }

        if ch == '_' || ch == '-' || ch == ' ' {
            if prev_was_word && !normalized.ends_with('_') {
                normalized.push('_');
            }
            prev_was_word = false;
            prev_was_lower_or_digit = false;
            continue;
        }

        return Err(anyhow!(
            "Broker input invalid: field '{field}' contains unsupported characters"
        ));
    }

    let collapsed = normalized.trim_matches('_').to_string();
    if collapsed.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' contains an empty value"
        ));
    }
    Ok(collapsed)
}

fn validate_enum_filter_values(
    values: Option<&[String]>,
    field: &str,
    allowed: &[&str],
) -> Result<()> {
    let Some(values) = values else {
        return Ok(());
    };

    for value in values {
        if !allowed.contains(&value.as_str()) {
            return Err(anyhow!(
                "Broker input invalid: field '{field}' value '{value}' is not supported. Valid values: {}",
                allowed.join(", ")
            ));
        }
    }

    Ok(())
}

fn format_enum_filter_help(prefix: &str, allowed: &[&str], normalization_hint: &str) -> String {
    format!("{prefix}: {}. {normalization_hint}", allowed.join(", "))
}

#[cfg(test)]
#[path = "broker_queries_tests.rs"]
mod tests;
