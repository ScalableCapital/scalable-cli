use anyhow::{Result, anyhow};
use chrono::{DateTime, FixedOffset, NaiveDate};
use serde_json::{Value, json};
use std::cmp::Ordering;

use crate::cli::{
    BrokerDerivativeIssuer, BrokerDerivativeKnockoutSubcategory, BrokerDerivativeSortField,
    BrokerDerivativeSortOrder, BrokerDerivativeStrategy, BrokerDerivativeType,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedBrokerDerivativesSearchQueryInput {
    pub(crate) derivative_type: BrokerDerivativeType,
    pub(crate) underlying_isin: String,
    pub(crate) limit: u16,
    pub(crate) offset: u32,
    pub(crate) issuers: Option<Vec<String>>,
    pub(crate) strategy: String,
    pub(crate) product_subcategories: Option<Vec<String>>,
    pub(crate) leverage_range: Option<NormalizedDecimalRange>,
    pub(crate) knockout_barrier_range: Option<NormalizedDecimalRange>,
    pub(crate) strike_range: Option<NormalizedDecimalRange>,
    pub(crate) omega_range: Option<NormalizedDecimalRange>,
    pub(crate) delta_range: Option<NormalizedSignedDecimalRange>,
    pub(crate) factor_range: Option<NormalizedDecimalRange>,
    pub(crate) expiry_date_range: Option<NormalizedDateRange>,
    pub(crate) sort_by: Option<NormalizedDerivativeSort>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedDecimalRange {
    pub(crate) min: Option<String>,
    pub(crate) max: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedSignedDecimalRange {
    pub(crate) min: Option<String>,
    pub(crate) max: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedDateRange {
    pub(crate) start_date: Option<String>,
    pub(crate) end_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedDerivativeSort {
    pub(crate) field: String,
    pub(crate) order: String,
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

pub const BROKER_DERIVATIVES_SEARCH_QUERY: &str = r#"
query BrokerDerivativesSearch(
  $accountId: ID!
  $portfolioId: ID!
  $input: DerivativeSearchInput!
) {
  account(id: $accountId) {
    brokerPortfolio(id: $portfolioId) {
      derivativesSearch(input: $input) {
        pagination {
          offset
          limit
          totalAvailable
        }
        results {
          __typename
          id
          isin
          underlyingIsin
          issuer
          ... on KnockoutSearchResult {
            premiumPercentage
            expiryDate {
              date {
                date
                epochDay
              }
              isOpenEnd
            }
            leverage
            knockoutBarrier {
              __typename
              ... on Money {
                currencyIsoCode
                value
              }
              ... on Point {
                value
              }
            }
            distanceToKnockout
            strike {
              __typename
              ... on Money {
                currencyIsoCode
                value
              }
              ... on Point {
                value
              }
            }
            distanceToStrike
            productSubcategory
            premiumAbsolute {
              currencyIsoCode
              value
            }
            strategy
          }
          ... on WarrantSearchResult {
            expiryDate {
              epochDay
            }
            strike {
              __typename
              ... on Money {
                currencyIsoCode
                value
              }
              ... on Point {
                value
              }
            }
            distanceToStrike
            strategy
            omega
            delta
            impliedVolatility
          }
          ... on FactorCertificateSearchResult {
            expiryDate {
              date {
                date
                epochDay
              }
              isOpenEnd
            }
            strategy
            factor
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
    let (sign, _, _) = parse_decimal_components(trimmed, field, false, "positive decimal")?;
    if sign <= 0 {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must be a positive decimal"
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_non_negative_decimal_with_field(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    let _ =
        parse_decimal_components(trimmed, field, false, "non-negative decimal").map_err(|_| {
            anyhow!("Broker input invalid: field '{field}' must be a non-negative decimal")
        })?;
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

pub(crate) fn normalize_broker_derivatives_search_query_input(
    args: &crate::cli::BrokerDerivativesSearchArgs,
) -> Result<NormalizedBrokerDerivativesSearchQueryInput> {
    let underlying_isin = args.underlying.trim();
    if underlying_isin.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field 'underlying' must be a non-empty string"
        ));
    }
    if args.offset > i32::MAX as u32 {
        return Err(anyhow!(
            "Broker input invalid: field 'offset' must be between 0 and {}",
            i32::MAX
        ));
    }

    let strategy = args.strategy;
    validate_derivative_strategy(args.derivative_type, strategy)?;

    let product_subcategories =
        normalize_derivative_product_subcategories(&args.product_subcategory);
    let leverage_range = normalize_positive_decimal_range(
        args.leverage_min.as_deref(),
        args.leverage_max.as_deref(),
        "leverage_min",
        "leverage_max",
    )?;
    let knockout_barrier_range = normalize_positive_decimal_range(
        args.knockout_barrier_min.as_deref(),
        args.knockout_barrier_max.as_deref(),
        "knockout_barrier_min",
        "knockout_barrier_max",
    )?;
    let strike_range = normalize_positive_decimal_range(
        args.strike_min.as_deref(),
        args.strike_max.as_deref(),
        "strike_min",
        "strike_max",
    )?;
    let omega_range = normalize_positive_decimal_range(
        args.omega_min.as_deref(),
        args.omega_max.as_deref(),
        "omega_min",
        "omega_max",
    )?;
    let delta_range = normalize_signed_decimal_range(
        args.delta_min.as_deref(),
        args.delta_max.as_deref(),
        "delta_min",
        "delta_max",
    )?;
    let factor_range = normalize_positive_decimal_range(
        args.factor_min.as_deref(),
        args.factor_max.as_deref(),
        "factor_min",
        "factor_max",
    )?;
    let expiry_date_range = normalize_date_range(
        args.expiry_from.as_deref(),
        args.expiry_to.as_deref(),
        "expiry_from",
        "expiry_to",
    )?;
    let sort_by = normalize_derivative_sort(args.sort_field, args.sort_order)?;

    match args.derivative_type {
        BrokerDerivativeType::Knockout => {
            reject_derivative_field_present(
                args.omega_min.is_some(),
                "omega_min",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.omega_max.is_some(),
                "omega_max",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.delta_min.is_some(),
                "delta_min",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.delta_max.is_some(),
                "delta_max",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.factor_min.is_some(),
                "factor_min",
                BrokerDerivativeType::Factor,
            )?;
            reject_derivative_field_present(
                args.factor_max.is_some(),
                "factor_max",
                BrokerDerivativeType::Factor,
            )?;
            validate_derivative_sort_field(
                args.derivative_type,
                args.sort_field,
                &[
                    BrokerDerivativeSortField::Strike,
                    BrokerDerivativeSortField::Leverage,
                    BrokerDerivativeSortField::ExpiryDate,
                    BrokerDerivativeSortField::KnockoutBarrier,
                    BrokerDerivativeSortField::DistanceToKnockout,
                    BrokerDerivativeSortField::PremiumAbsolute,
                    BrokerDerivativeSortField::PremiumRelative,
                ],
            )?;
        }
        BrokerDerivativeType::Warrant => {
            reject_derivative_field_present(
                !args.product_subcategory.is_empty(),
                "product_subcategory",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.leverage_min.is_some(),
                "leverage_min",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.leverage_max.is_some(),
                "leverage_max",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.knockout_barrier_min.is_some(),
                "knockout_barrier_min",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.knockout_barrier_max.is_some(),
                "knockout_barrier_max",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.factor_min.is_some(),
                "factor_min",
                BrokerDerivativeType::Factor,
            )?;
            reject_derivative_field_present(
                args.factor_max.is_some(),
                "factor_max",
                BrokerDerivativeType::Factor,
            )?;
            validate_derivative_sort_field(
                args.derivative_type,
                args.sort_field,
                &[
                    BrokerDerivativeSortField::Strike,
                    BrokerDerivativeSortField::DistanceToStrike,
                    BrokerDerivativeSortField::Omega,
                    BrokerDerivativeSortField::Delta,
                    BrokerDerivativeSortField::ExpiryDate,
                    BrokerDerivativeSortField::ImpliedVolatility,
                ],
            )?;
        }
        BrokerDerivativeType::Factor => {
            reject_derivative_field_present(
                !args.product_subcategory.is_empty(),
                "product_subcategory",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.leverage_min.is_some(),
                "leverage_min",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.leverage_max.is_some(),
                "leverage_max",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.knockout_barrier_min.is_some(),
                "knockout_barrier_min",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.knockout_barrier_max.is_some(),
                "knockout_barrier_max",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.omega_min.is_some(),
                "omega_min",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.omega_max.is_some(),
                "omega_max",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.delta_min.is_some(),
                "delta_min",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.delta_max.is_some(),
                "delta_max",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.expiry_from.is_some(),
                "expiry_from",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.expiry_to.is_some(),
                "expiry_to",
                BrokerDerivativeType::Warrant,
            )?;
            reject_derivative_field_present(
                args.strike_min.is_some(),
                "strike_min",
                BrokerDerivativeType::Knockout,
            )?;
            reject_derivative_field_present(
                args.strike_max.is_some(),
                "strike_max",
                BrokerDerivativeType::Knockout,
            )?;
            validate_derivative_sort_field(
                args.derivative_type,
                args.sort_field,
                &[
                    BrokerDerivativeSortField::Factor,
                    BrokerDerivativeSortField::ExpiryDate,
                ],
            )?;
        }
    }

    Ok(NormalizedBrokerDerivativesSearchQueryInput {
        derivative_type: args.derivative_type,
        underlying_isin: underlying_isin.to_string(),
        limit: args.limit,
        offset: args.offset,
        issuers: normalize_derivative_issuers(&args.issuer),
        strategy: strategy.as_graphql().to_string(),
        product_subcategories,
        leverage_range,
        knockout_barrier_range,
        strike_range,
        omega_range,
        delta_range,
        factor_range,
        expiry_date_range,
        sort_by,
    })
}

pub fn broker_derivatives_search_variables(
    input: &BrokerInput,
    query: &NormalizedBrokerDerivativesSearchQueryInput,
) -> Result<Value> {
    let mut derivatives_input = serde_json::Map::new();
    derivatives_input.insert("knockoutInput".to_string(), Value::Null);
    derivatives_input.insert("warrantInput".to_string(), Value::Null);
    derivatives_input.insert("factorCertificateInput".to_string(), Value::Null);

    match query.derivative_type {
        BrokerDerivativeType::Knockout => {
            derivatives_input.insert(
                "knockoutInput".to_string(),
                Value::Object(build_knockout_derivatives_input(query)),
            );
        }
        BrokerDerivativeType::Warrant => {
            derivatives_input.insert(
                "warrantInput".to_string(),
                Value::Object(build_warrant_derivatives_input(query)),
            );
        }
        BrokerDerivativeType::Factor => {
            derivatives_input.insert(
                "factorCertificateInput".to_string(),
                Value::Object(build_factor_derivatives_input(query)),
            );
        }
    }

    Ok(json!({
        "accountId": input.account_id,
        "portfolioId": input.portfolio_id,
        "input": Value::Object(derivatives_input),
    }))
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

fn build_knockout_derivatives_input(
    query: &NormalizedBrokerDerivativesSearchQueryInput,
) -> serde_json::Map<String, Value> {
    let mut input = derivative_input_base(query);
    input.insert(
        "strategy".to_string(),
        Value::String(query.strategy.clone()),
    );
    insert_optional_string_array(&mut input, "issuers", query.issuers.as_ref());
    insert_optional_string_array(
        &mut input,
        "productSubcategories",
        query.product_subcategories.as_ref(),
    );
    insert_optional_range(&mut input, "leverageRange", query.leverage_range.as_ref());
    insert_optional_range(
        &mut input,
        "knockoutBarrier",
        query.knockout_barrier_range.as_ref(),
    );
    insert_optional_range(&mut input, "strike", query.strike_range.as_ref());
    insert_optional_knockout_expiry_date(&mut input, query.expiry_date_range.as_ref());
    insert_optional_sort(&mut input, query.sort_by.as_ref());
    input
}

fn build_warrant_derivatives_input(
    query: &NormalizedBrokerDerivativesSearchQueryInput,
) -> serde_json::Map<String, Value> {
    let mut input = derivative_input_base(query);
    input.insert(
        "strategy".to_string(),
        Value::String(query.strategy.clone()),
    );
    insert_optional_string_array(&mut input, "issuers", query.issuers.as_ref());
    insert_optional_range(&mut input, "strikeRange", query.strike_range.as_ref());
    insert_optional_range(&mut input, "omegaRange", query.omega_range.as_ref());
    insert_optional_signed_range(&mut input, "deltaRange", query.delta_range.as_ref());
    insert_optional_warrant_expiry_date(&mut input, query.expiry_date_range.as_ref());
    insert_optional_sort(&mut input, query.sort_by.as_ref());
    input
}

fn build_factor_derivatives_input(
    query: &NormalizedBrokerDerivativesSearchQueryInput,
) -> serde_json::Map<String, Value> {
    let mut input = derivative_input_base(query);
    input.insert(
        "strategy".to_string(),
        Value::String(query.strategy.clone()),
    );
    insert_optional_string_array(&mut input, "issuers", query.issuers.as_ref());
    insert_optional_range(&mut input, "factorRange", query.factor_range.as_ref());
    insert_optional_sort(&mut input, query.sort_by.as_ref());
    input
}

fn derivative_input_base(
    query: &NormalizedBrokerDerivativesSearchQueryInput,
) -> serde_json::Map<String, Value> {
    let mut input = serde_json::Map::new();
    input.insert(
        "underlyingIsin".to_string(),
        Value::String(query.underlying_isin.clone()),
    );
    input.insert(
        "pagination".to_string(),
        json!({
            "offset": query.offset,
            "limit": query.limit,
        }),
    );
    input
}

fn insert_optional_string_array(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    values: Option<&Vec<String>>,
) {
    if let Some(values) = values {
        object.insert(
            key.to_string(),
            Value::Array(values.iter().cloned().map(Value::String).collect()),
        );
    }
}

fn insert_optional_range(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    range: Option<&NormalizedDecimalRange>,
) {
    if let Some(range) = range {
        object.insert(
            key.to_string(),
            json!({
                "min": range.min,
                "max": range.max,
            }),
        );
    }
}

fn insert_optional_signed_range(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    range: Option<&NormalizedSignedDecimalRange>,
) {
    if let Some(range) = range {
        object.insert(
            key.to_string(),
            json!({
                "min": range.min,
                "max": range.max,
            }),
        );
    }
}

fn insert_optional_warrant_expiry_date(
    object: &mut serde_json::Map<String, Value>,
    range: Option<&NormalizedDateRange>,
) {
    if let Some(range) = range {
        object.insert(
            "expiryDate".to_string(),
            json!({
                "startDate": range.start_date,
                "endDate": range.end_date,
            }),
        );
    }
}

fn insert_optional_knockout_expiry_date(
    object: &mut serde_json::Map<String, Value>,
    range: Option<&NormalizedDateRange>,
) {
    if let Some(range) = range {
        object.insert(
            "expiryDate".to_string(),
            json!({
                "startDate": range.start_date,
                "endDate": range.end_date,
                "isOpenEnd": false,
            }),
        );
    }
}

fn insert_optional_sort(
    object: &mut serde_json::Map<String, Value>,
    sort: Option<&NormalizedDerivativeSort>,
) {
    if let Some(sort) = sort {
        object.insert(
            "sortBy".to_string(),
            json!({
                "field": sort.field,
                "order": sort.order,
            }),
        );
    }
}

fn normalize_derivative_issuers(values: &[BrokerDerivativeIssuer]) -> Option<Vec<String>> {
    if values.is_empty() {
        return None;
    }

    let mut normalized = values
        .iter()
        .map(|value| value.as_graphql().to_string())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    Some(normalized)
}

fn normalize_derivative_product_subcategories(
    values: &[BrokerDerivativeKnockoutSubcategory],
) -> Option<Vec<String>> {
    if values.is_empty() {
        return None;
    }

    let mut normalized = values
        .iter()
        .map(|value| value.as_graphql().to_string())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    Some(normalized)
}

fn normalize_positive_decimal_range(
    min: Option<&str>,
    max: Option<&str>,
    min_field: &str,
    max_field: &str,
) -> Result<Option<NormalizedDecimalRange>> {
    let min = min
        .map(|value| normalize_positive_decimal_with_field(value, min_field))
        .transpose()?;
    let max = max
        .map(|value| normalize_positive_decimal_with_field(value, max_field))
        .transpose()?;

    validate_decimal_range_order(min.as_deref(), max.as_deref(), min_field, max_field)?;

    if min.is_none() && max.is_none() {
        Ok(None)
    } else {
        Ok(Some(NormalizedDecimalRange { min, max }))
    }
}

fn normalize_signed_decimal_range(
    min: Option<&str>,
    max: Option<&str>,
    min_field: &str,
    max_field: &str,
) -> Result<Option<NormalizedSignedDecimalRange>> {
    let min = min
        .map(|value| normalize_decimal_with_field(value, min_field))
        .transpose()?;
    let max = max
        .map(|value| normalize_decimal_with_field(value, max_field))
        .transpose()?;

    validate_decimal_range_order(min.as_deref(), max.as_deref(), min_field, max_field)?;

    if min.is_none() && max.is_none() {
        Ok(None)
    } else {
        Ok(Some(NormalizedSignedDecimalRange { min, max }))
    }
}

fn normalize_date_range(
    start_date: Option<&str>,
    end_date: Option<&str>,
    start_field: &str,
    end_field: &str,
) -> Result<Option<NormalizedDateRange>> {
    let start_date = start_date
        .map(|value| normalize_local_date(value, start_field))
        .transpose()?;
    let end_date = end_date
        .map(|value| normalize_local_date(value, end_field))
        .transpose()?;

    if let (Some(start_date), Some(end_date)) = (start_date.as_deref(), end_date.as_deref()) {
        let start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d").map_err(|_| {
            anyhow!("Broker input invalid: field '{start_field}' must use YYYY-MM-DD format")
        })?;
        let end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d").map_err(|_| {
            anyhow!("Broker input invalid: field '{end_field}' must use YYYY-MM-DD format")
        })?;
        if start > end {
            return Err(anyhow!(
                "Broker input invalid: fields '{start_field}' and '{end_field}' must define a valid range"
            ));
        }
    }

    if start_date.is_none() && end_date.is_none() {
        Ok(None)
    } else {
        Ok(Some(NormalizedDateRange {
            start_date,
            end_date,
        }))
    }
}

fn normalize_derivative_sort(
    field: Option<BrokerDerivativeSortField>,
    order: Option<BrokerDerivativeSortOrder>,
) -> Result<Option<NormalizedDerivativeSort>> {
    match (field, order) {
        (None, None) => Ok(None),
        (Some(field), Some(order)) => Ok(Some(NormalizedDerivativeSort {
            field: field.as_graphql().to_string(),
            order: order.as_graphql().to_string(),
        })),
        (Some(_), None) => Err(anyhow!(
            "Broker input invalid: field 'sort_order' is required when 'sort_field' is set"
        )),
        (None, Some(_)) => Err(anyhow!(
            "Broker input invalid: field 'sort_field' is required when 'sort_order' is set"
        )),
    }
}

fn validate_derivative_strategy(
    derivative_type: BrokerDerivativeType,
    strategy: BrokerDerivativeStrategy,
) -> Result<()> {
    let valid = match derivative_type {
        BrokerDerivativeType::Knockout | BrokerDerivativeType::Factor => {
            matches!(
                strategy,
                BrokerDerivativeStrategy::Long | BrokerDerivativeStrategy::Short
            )
        }
        BrokerDerivativeType::Warrant => {
            matches!(
                strategy,
                BrokerDerivativeStrategy::Call | BrokerDerivativeStrategy::Put
            )
        }
    };

    if valid {
        Ok(())
    } else {
        Err(anyhow!(
            "Broker input invalid: strategy '{:?}' is not supported for derivative type '{}'",
            strategy,
            derivative_type.as_label()
        ))
    }
}

fn validate_derivative_sort_field(
    derivative_type: BrokerDerivativeType,
    field: Option<BrokerDerivativeSortField>,
    allowed: &[BrokerDerivativeSortField],
) -> Result<()> {
    let Some(field) = field else {
        return Ok(());
    };

    if allowed.contains(&field) {
        Ok(())
    } else {
        Err(anyhow!(
            "Broker input invalid: sort field '{}' is not supported for derivative type '{}'",
            field.as_graphql(),
            derivative_type.as_label()
        ))
    }
}

fn reject_derivative_field_present(
    is_present: bool,
    field: &str,
    _supported_type: BrokerDerivativeType,
) -> Result<()> {
    if is_present {
        Err(anyhow!(
            "Broker input invalid: field '{field}' is not supported for the selected derivative type"
        ))
    } else {
        Ok(())
    }
}

fn normalize_decimal_with_field(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    let _ = parse_decimal_components(trimmed, field, true, "decimal")?;
    Ok(trimmed.to_string())
}

fn validate_decimal_range_order(
    min: Option<&str>,
    max: Option<&str>,
    min_field: &str,
    max_field: &str,
) -> Result<()> {
    let (Some(min), Some(max)) = (min, max) else {
        return Ok(());
    };
    let min_value = parse_decimal_components(min, min_field, true, "decimal")?;
    let max_value = parse_decimal_components(max, max_field, true, "decimal")?;
    if compare_decimal_components(&min_value, &max_value) == Ordering::Greater {
        return Err(anyhow!(
            "Broker input invalid: fields '{min_field}' and '{max_field}' must define a valid range"
        ));
    }
    Ok(())
}

fn normalize_local_date(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must use YYYY-MM-DD format"
        ));
    }
    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .map_err(|_| anyhow!("Broker input invalid: field '{field}' must use YYYY-MM-DD format"))?;
    Ok(trimmed.to_string())
}

fn parse_decimal_components(
    raw: &str,
    field: &str,
    allow_negative: bool,
    kind: &str,
) -> Result<(i8, String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must be a {kind}"
        ));
    }

    let (negative, unsigned) = match trimmed.strip_prefix('-') {
        Some(rest) if allow_negative => (true, rest),
        Some(_) => {
            return Err(anyhow!(
                "Broker input invalid: field '{field}' must be a {kind}"
            ));
        }
        None => match trimmed.strip_prefix('+') {
            Some(rest) => (false, rest),
            None => (false, trimmed),
        },
    };

    let mut parts = unsigned.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !fraction.chars().all(|c| c.is_ascii_digit())
        || (whole.is_empty() && fraction.is_empty())
    {
        return Err(anyhow!(
            "Broker input invalid: field '{field}' must be a {kind}"
        ));
    }

    let normalized_whole = {
        let stripped = whole.trim_start_matches('0');
        if stripped.is_empty() {
            "0".to_string()
        } else {
            stripped.to_string()
        }
    };
    let normalized_fraction = fraction.trim_end_matches('0').to_string();
    let sign = if normalized_whole == "0" && normalized_fraction.is_empty() {
        0
    } else if negative {
        -1
    } else {
        1
    };

    Ok((sign, normalized_whole, normalized_fraction))
}

fn compare_decimal_components(
    left: &(i8, String, String),
    right: &(i8, String, String),
) -> Ordering {
    if left.0 != right.0 {
        return left.0.cmp(&right.0);
    }
    if left.0 == 0 {
        return Ordering::Equal;
    }

    let abs = compare_decimal_abs(&left.1, &left.2, &right.1, &right.2);
    if left.0 > 0 { abs } else { abs.reverse() }
}

fn compare_decimal_abs(
    left_whole: &str,
    left_fraction: &str,
    right_whole: &str,
    right_fraction: &str,
) -> Ordering {
    match left_whole.len().cmp(&right_whole.len()) {
        Ordering::Equal => {}
        ordering => return ordering,
    }
    match left_whole.cmp(right_whole) {
        Ordering::Equal => {}
        ordering => return ordering,
    }

    let max_fraction_len = left_fraction.len().max(right_fraction.len());
    for index in 0..max_fraction_len {
        let left_digit = left_fraction.as_bytes().get(index).copied().unwrap_or(b'0');
        let right_digit = right_fraction
            .as_bytes()
            .get(index)
            .copied()
            .unwrap_or(b'0');
        match left_digit.cmp(&right_digit) {
            Ordering::Equal => continue,
            ordering => return ordering,
        }
    }
    Ordering::Equal
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
