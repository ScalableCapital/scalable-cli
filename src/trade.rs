use anyhow::{Result, anyhow, bail};
use serde_json::map::Entry;
use serde_json::{Value, json};

pub const TRADE_TRADABILITY_QUERY: &str = r#"
query getTradingTradability($personId: ID!, $isin: ID!, $portfolioId: ID!) {
  account(id: $personId) {
    id
    brokerPortfolio(id: $portfolioId) {
      id
      security(isin: $isin) {
        id
        buyTradabilityForTrading {
          id
          tradabilityStatus
          venues {
            venue
            tradabilityStatus
            unavailabilityReason
          }
          primaryVenue {
            venue
            status
          }
        }
        sellTradabilityForTrading {
          id
          tradabilityStatus
          venues {
            venue
            tradabilityStatus
            unavailabilityReason
          }
          primaryVenue {
            venue
            status
          }
        }
        inventory {
          position {
            sellableByVenue {
              venue
              sellable
            }
          }
        }
      }
    }
  }
}
"#;

pub const TRADE_APPROPRIATENESS_RESULT_QUERY: &str = r#"
query getAppropriatenessResult($personId: ID!, $portfolioId: ID!) {
  account(id: $personId) {
    id
    brokerPortfolio(id: $portfolioId) {
      id
      appropriatenessInfo {
        id
        appropriatenessId
        result
      }
    }
  }
}
"#;

pub const TRADE_APPROPRIATENESS_WARNING_QUERY: &str = r#"
query getBrokerAppropriatenessWarning($locale: String!) {
  brokerAppropriatenessWarning(locale: $locale) {
    id
    version
    locale
    promptText
    acknowledgementText
  }
}
"#;

pub const TRADE_SECURITY_TICK_QUERY: &str = r#"
query getSecurityTick(
  $personId: ID!
  $isin: ID!
  $portfolioId: ID!
  $includeYearToDate: Boolean
  $source: MarketDataSource
) {
  account(id: $personId) {
    id
    brokerPortfolio(id: $portfolioId) {
      id
      security(isin: $isin) {
        id
        isin
        quoteTick(source: $source, includeYearToDate: $includeYearToDate) {
          askPrice
          bidPrice
          midPrice
          currency
          isOutdated
          timestampUtc {
            time
          }
        }
        issuerLinks {
          kidLinks {
            isPrimary
            url
            locale
          }
        }
      }
    }
  }
}
"#;

pub const TRADE_SINGLE_EX_ANTE_COSTS_QUERY: &str = r#"
query getSingleTradeExAnteCost(
  $personId: ID!
  $isin: String!
  $side: OrderSide!
  $estimatedOrderVolume: BigDecimal!
  $numberOfShares: BigDecimal!
  $venue: TradingVenue!
  $isWholePositionSold: Boolean!
  $portfolioId: ID!
) {
  account(id: $personId) {
    id
    brokerPortfolio(id: $portfolioId) {
      id
      singleTradeExAnteCosts(
        input: {
          isin: $isin
          side: $side
          venue: $venue
          estimatedOrderVolume: $estimatedOrderVolume
          numberOfShares: $numberOfShares
          isWholePositionSold: $isWholePositionSold
        }
      ) {
        id
        entryCosts {
          productCosts {
            amount
            percentage
          }
          serviceCosts {
            amount
            percentage
          }
          total {
            amount
            percentage
          }
        }
        ongoingCosts {
          productCosts {
            amount
            percentage
          }
          serviceCosts {
            amount
            percentage
          }
          total {
            amount
            percentage
          }
        }
        exitCosts {
          productCosts {
            amount
            percentage
          }
          serviceCosts {
            amount
            percentage
          }
          total {
            amount
            percentage
          }
        }
        effectOnReturn {
          initialYearCosts {
            amount
            percentage
          }
          followingYearsCosts {
            amount
            percentage
          }
          finalYearCosts {
            amount
            percentage
          }
        }
        fiveYearsCosts {
          amount
          percentage
        }
        incidentalCosts {
          amount
          percentage
        }
      }
    }
  }
}
"#;

pub const TRADE_FILL_FORECAST_MUTATION: &str = r#"
mutation createFillForecast($portfolioId: ID!, $input: FillForecastInput!) {
  createFillForecast(portfolioId: $portfolioId, input: $input) {
    statusCategory
    fillForecastResponse {
      id
      limitFillProbabilities
      limitRelativePrices
      stopFillProbabilities
      stopRelativePrices
      validUntil {
        epochSecond
      }
    }
  }
}
"#;

pub const TRADE_PLACE_ORDER_MUTATION: &str = r#"
mutation placeOrder($input: BrokerOrderInput!, $portfolioId: ID!, $isBuy: Boolean!) {
  placeOrder(portfolioId: $portfolioId, input: $input) {
    brokerPortfolio {
      id
    }
    orderData {
      orderId
      isMarketable @include(if: $isBuy)
    }
  }
}
"#;

pub const TRADE_CANCEL_ORDER_MUTATION: &str = r#"
mutation BrokerCancelOrder($portfolioId: ID!, $orderId: ID!) {
  cancelOrder(portfolioId: $portfolioId, input: { orderId: $orderId }) {
    id
  }
}
"#;

pub(crate) fn required_non_empty(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Trade input invalid: field '{}' must be a non-empty string",
            field
        ));
    }
    Ok(trimmed.to_string())
}

fn required_positive_decimal(value: f64, field: &str) -> Result<f64> {
    if !value.is_finite() || value <= 0.0 {
        return Err(anyhow!(
            "Trade input invalid: field '{}' must be a positive decimal",
            field
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TradeTradabilityGate {
    pub status: String,
    pub tradable: bool,
    pub requires_appropriateness: bool,
    pub selected_venue: String,
    pub selected_venue_status: String,
    pub selected_venue_unavailability_reason: Option<String>,
    pub selected_venue_sellable: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeSide {
    Buy,
    Sell,
}

impl TradeSide {
    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }

    pub fn is_buy(self) -> bool {
        matches!(self, Self::Buy)
    }

    fn tradability_field(self) -> &'static str {
        match self {
            Self::Buy => "buyTradabilityForTrading",
            Self::Sell => "sellTradabilityForTrading",
        }
    }

    pub fn as_label(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppropriatenessGate {
    pub status: String,
    pub appropriateness_id: Option<String>,
    pub passed: bool,
    pub requires_warning_ack: bool,
    pub requires_questionnaire: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuitabilityReview {
    pub status: String,
    pub is_suitable: bool,
    pub requires_accept_unsuitable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppropriatenessWarning {
    pub version: String,
    pub locale: String,
    pub prompt_text: String,
    pub acknowledgement_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecurityTick {
    pub ask_price: Option<f64>,
    pub bid_price: Option<f64>,
    pub mid_price: f64,
    pub currency: String,
    pub is_outdated: bool,
    pub timestamp_utc: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TradePriceSelection {
    pub basis: &'static str,
    pub price: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecurityIssuerDocumentLinks {
    pub primary_kid_url: Option<String>,
    pub secondary_kid_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaceOrderResult {
    pub order_id: String,
    pub is_marketable: Option<bool>,
}

pub fn parse_tradability_gate(
    response: &Value,
    side: TradeSide,
    venue_override: Option<&str>,
) -> Result<TradeTradabilityGate> {
    let tradability = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("security"))
        .and_then(|v| v.get(side.tradability_field()))
        .ok_or_else(|| {
            anyhow!(
                "Trade response invalid: missing {} tradability block",
                side.as_label()
            )
        })?;

    let status = tradability
        .get("tradabilityStatus")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Trade response invalid: missing tradability status"))?
        .to_string();

    let venues = tradability
        .get("venues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let selected_venue = if let Some(override_venue) = venue_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if venues
            .iter()
            .any(|item| item.get("venue").and_then(Value::as_str) == Some(override_venue))
        {
            override_venue.to_string()
        } else {
            return Err(anyhow!(
                "Trade input invalid: venue '{}' not available for this security",
                override_venue
            ));
        }
    } else if let Some(primary_venue) = tradability
        .get("primaryVenue")
        .and_then(|v| v.get("venue"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        primary_venue.to_string()
    } else {
        venues
            .iter()
            .filter_map(|item| item.get("venue").and_then(Value::as_str))
            .find(|value| !value.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| anyhow!("Trade response invalid: no tradable venue available"))?
    };

    let selected_venue_entry = venues
        .iter()
        .find(|item| item.get("venue").and_then(Value::as_str) == Some(selected_venue.as_str()));
    let selected_venue_status = selected_venue_entry
        .and_then(|item| item.get("tradabilityStatus"))
        .and_then(Value::as_str)
        .unwrap_or(status.as_str())
        .to_string();
    let selected_venue_unavailability_reason = selected_venue_entry
        .and_then(|item| item.get("unavailabilityReason"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let tradable = status != "NOT_TRADABLE" && selected_venue_status != "NOT_TRADABLE";
    let requires_appropriateness = status == "TRADABLE_WITH_APPROPRIATENESS"
        || selected_venue_status == "TRADABLE_WITH_APPROPRIATENESS";

    let selected_venue_sellable = if side == TradeSide::Sell {
        let sellable_by_venue = response
            .get("account")
            .and_then(|v| v.get("brokerPortfolio"))
            .and_then(|v| v.get("security"))
            .and_then(|v| v.get("inventory"))
            .and_then(|v| v.get("position"))
            .and_then(|v| v.get("sellableByVenue"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow!(
                    "Trade response invalid: missing sellableByVenue for {} tradability",
                    side.as_label()
                )
            })?;
        let sellable = sellable_by_venue
            .iter()
            .find(|item| item.get("venue").and_then(Value::as_str) == Some(selected_venue.as_str()))
            .and_then(|item| item.get("sellable"))
            .and_then(Value::as_f64)
            .ok_or_else(|| {
                anyhow!(
                    "Trade response invalid: missing sellable quantity for selected venue '{}'",
                    selected_venue
                )
            })?;
        Some(sellable)
    } else {
        None
    };

    Ok(TradeTradabilityGate {
        status,
        tradable,
        requires_appropriateness,
        selected_venue,
        selected_venue_status,
        selected_venue_unavailability_reason,
        selected_venue_sellable,
    })
}

pub fn evaluate_appropriateness_gate(
    response: &Value,
    appropriateness_required: bool,
) -> Result<AppropriatenessGate> {
    if !appropriateness_required {
        return Ok(AppropriatenessGate {
            status: "NOT_REQUIRED".to_string(),
            appropriateness_id: None,
            passed: true,
            requires_warning_ack: false,
            requires_questionnaire: false,
        });
    }

    let info = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("appropriatenessInfo"))
        .ok_or_else(|| anyhow!("Trade response invalid: missing appropriateness info"))?;

    let status = info
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Trade response invalid: missing appropriateness result"))?
        .to_string();
    let appropriateness_id = info
        .get("appropriatenessId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    Ok(AppropriatenessGate {
        passed: status == "APPROPRIATE",
        requires_warning_ack: status == "NOT_APPROPRIATE",
        requires_questionnaire: status == "NOT_EVALUATED",
        status,
        appropriateness_id,
    })
}

pub fn parse_appropriateness_warning(response: &Value) -> Result<AppropriatenessWarning> {
    let warning = response
        .get("brokerAppropriatenessWarning")
        .ok_or_else(|| anyhow!("Trade response invalid: missing broker appropriateness warning"))?;

    Ok(AppropriatenessWarning {
        version: warning
            .get("version")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("Trade response invalid: missing warning version"))?
            .to_string(),
        locale: warning
            .get("locale")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("Trade response invalid: missing warning locale"))?
            .to_string(),
        prompt_text: warning
            .get("promptText")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("Trade response invalid: missing warning prompt text"))?
            .to_string(),
        acknowledgement_text: warning
            .get("acknowledgementText")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("Trade response invalid: missing warning acknowledgement text"))?
            .to_string(),
    })
}

pub fn evaluate_suitability(status: Option<&str>) -> SuitabilityReview {
    let status = status.unwrap_or("UNKNOWN").to_string();
    let (is_suitable, requires_accept_unsuitable) = match status.as_str() {
        "APPROPRIATE" | "NOT_REQUIRED" => (true, false),
        _ => (false, true),
    };

    SuitabilityReview {
        status,
        is_suitable,
        requires_accept_unsuitable,
    }
}

pub fn trade_security_tick_variables(
    person_id: &str,
    portfolio_id: &str,
    isin: &str,
) -> Result<Value> {
    Ok(json!({
        "personId": required_non_empty(person_id, "person_id")?,
        "portfolioId": required_non_empty(portfolio_id, "portfolio_id")?,
        "isin": required_non_empty(isin, "isin")?,
        "includeYearToDate": false,
        "source": "CONSOLIDATED",
    }))
}

pub fn parse_security_tick(response: &Value) -> Result<SecurityTick> {
    let tick = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("security"))
        .and_then(|v| v.get("quoteTick"))
        .ok_or_else(|| anyhow!("Trade response invalid: missing security quote tick"))?;

    let ask_price = parse_optional_quote_leg(tick, "askPrice")?;
    let bid_price = parse_optional_quote_leg(tick, "bidPrice")?;
    let mid_price = tick
        .get("midPrice")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("Trade response invalid: missing security quote midPrice"))?;
    if !mid_price.is_finite() || mid_price <= 0.0 {
        return Err(anyhow!(
            "EX_ANTE_COST_UNAVAILABLE: invalid quote midPrice '{}' for amount-to-shares calculation",
            mid_price
        ));
    }

    let currency = tick
        .get("currency")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Trade response invalid: missing security quote currency"))?
        .to_string();
    let is_outdated = tick
        .get("isOutdated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let timestamp_utc = tick
        .get("timestampUtc")
        .and_then(|v| v.get("time"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    Ok(SecurityTick {
        ask_price,
        bid_price,
        mid_price,
        currency,
        is_outdated,
        timestamp_utc,
    })
}

fn parse_optional_quote_leg(tick: &Value, field: &str) -> Result<Option<f64>> {
    let Some(value) = tick.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let parsed = value
        .as_f64()
        .ok_or_else(|| anyhow!("Trade response invalid: security quote {field} is not numeric"))?;
    if !parsed.is_finite() || parsed < 0.0 {
        bail!("Trade response invalid: invalid security quote {field} '{parsed}'");
    }
    if parsed == 0.0 {
        return Ok(None);
    }
    Ok(Some(parsed))
}

pub fn parse_security_issuer_document_links(
    response: &Value,
    locale: &str,
) -> Result<SecurityIssuerDocumentLinks> {
    let security = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("security"))
        .ok_or_else(|| anyhow!("Trade response invalid: missing security block"))?;

    let kid_links = security
        .get("issuerLinks")
        .and_then(|v| v.get("kidLinks"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let primary_kid_url = kid_links
        .iter()
        .filter(|entry| entry.get("isPrimary").and_then(Value::as_bool) == Some(true))
        .find_map(|entry| {
            entry
                .get("url")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToString::to_string)
        });

    let secondary_kid_url = kid_links
        .iter()
        .filter(|entry| {
            entry.get("isPrimary").and_then(Value::as_bool) == Some(false)
                && entry.get("locale").and_then(Value::as_str) == Some(locale)
        })
        .find_map(|entry| {
            entry
                .get("url")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToString::to_string)
        });

    Ok(SecurityIssuerDocumentLinks {
        primary_kid_url,
        secondary_kid_url,
    })
}

pub fn extract_single_trade_ex_ante_costs(response: &Value) -> Result<Value> {
    let mut costs = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("singleTradeExAnteCosts"))
        .ok_or_else(|| anyhow!("EX_ANTE_COST_UNAVAILABLE: missing singleTradeExAnteCosts"))?
        .clone();

    normalize_single_trade_ex_ante_costs(&mut costs)?;
    Ok(costs)
}

#[derive(Clone, Copy)]
struct NullableExAnteNumericGroup {
    root_path: &'static str,
    numeric_paths: &'static [&'static str],
}

const OPTIONAL_ENTRY_EX_ANTE_NUMERIC_PATHS: [&str; 6] = [
    "/entryCosts/serviceCosts/amount",
    "/entryCosts/serviceCosts/percentage",
    "/entryCosts/productCosts/amount",
    "/entryCosts/productCosts/percentage",
    "/entryCosts/total/amount",
    "/entryCosts/total/percentage",
];

const OPTIONAL_INITIAL_YEAR_EX_ANTE_NUMERIC_PATHS: [&str; 2] = [
    "/effectOnReturn/initialYearCosts/amount",
    "/effectOnReturn/initialYearCosts/percentage",
];

const OPTIONAL_FOLLOWING_YEARS_EX_ANTE_NUMERIC_PATHS: [&str; 2] = [
    "/effectOnReturn/followingYearsCosts/amount",
    "/effectOnReturn/followingYearsCosts/percentage",
];

const NULLABLE_EX_ANTE_NUMERIC_GROUPS: [NullableExAnteNumericGroup; 3] = [
    NullableExAnteNumericGroup {
        root_path: "/entryCosts",
        numeric_paths: &OPTIONAL_ENTRY_EX_ANTE_NUMERIC_PATHS,
    },
    NullableExAnteNumericGroup {
        root_path: "/effectOnReturn/initialYearCosts",
        numeric_paths: &OPTIONAL_INITIAL_YEAR_EX_ANTE_NUMERIC_PATHS,
    },
    NullableExAnteNumericGroup {
        root_path: "/effectOnReturn/followingYearsCosts",
        numeric_paths: &OPTIONAL_FOLLOWING_YEARS_EX_ANTE_NUMERIC_PATHS,
    },
];

const REQUIRED_EX_ANTE_NUMERIC_PATHS: [&str; 14] = [
    "/ongoingCosts/serviceCosts/amount",
    "/ongoingCosts/serviceCosts/percentage",
    "/ongoingCosts/productCosts/amount",
    "/ongoingCosts/productCosts/percentage",
    "/ongoingCosts/total/amount",
    "/ongoingCosts/total/percentage",
    "/exitCosts/serviceCosts/amount",
    "/exitCosts/serviceCosts/percentage",
    "/exitCosts/productCosts/amount",
    "/exitCosts/productCosts/percentage",
    "/exitCosts/total/amount",
    "/exitCosts/total/percentage",
    "/effectOnReturn/finalYearCosts/amount",
    "/effectOnReturn/finalYearCosts/percentage",
];

fn normalize_single_trade_ex_ante_costs(costs: &mut Value) -> Result<()> {
    for path in REQUIRED_EX_ANTE_NUMERIC_PATHS {
        ensure_numeric_path(costs, path)?;
    }
    for group in NULLABLE_EX_ANTE_NUMERIC_GROUPS {
        normalize_nullable_ex_ante_numeric_group(costs, group)?;
    }
    Ok(())
}

fn normalize_nullable_ex_ante_numeric_group(
    costs: &mut Value,
    group: NullableExAnteNumericGroup,
) -> Result<()> {
    match costs.pointer(group.root_path) {
        Some(value) if value.is_null() => Ok(()),
        Some(value) if value.is_object() => {
            for path in group.numeric_paths {
                ensure_numeric_path(costs, path)?;
            }
            Ok(())
        }
        Some(_) => bail!(
            "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '{}'",
            group.root_path
        ),
        None => bail!(
            "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '{}'",
            group.root_path
        ),
    }
}

fn ensure_numeric_path(value: &mut Value, path: &str) -> Result<()> {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    ensure_numeric_path_segments(value, &segments, "")
}

fn ensure_numeric_path_segments(
    value: &mut Value,
    segments: &[&str],
    current_path: &str,
) -> Result<()> {
    if segments.is_empty() {
        return Ok(());
    }

    let object = value.as_object_mut().ok_or_else(|| {
        anyhow!(
            "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '{}'",
            display_ex_ante_path(current_path)
        )
    })?;

    if segments.len() == 1 {
        object
            .entry(segments[0].to_string())
            .or_insert_with(|| json!(0));
        return Ok(());
    }

    let next_path = join_ex_ante_path(current_path, segments[0]);
    match object.entry(segments[0].to_string()) {
        Entry::Occupied(entry) => {
            if !entry.get().is_object() {
                return Err(anyhow!(
                    "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '{}'",
                    next_path
                ));
            }
            ensure_numeric_path_segments(entry.into_mut(), &segments[1..], &next_path)
        }
        Entry::Vacant(entry) => {
            let child = entry.insert(json!({}));
            ensure_numeric_path_segments(child, &segments[1..], &next_path)
        }
    }
}

fn join_ex_ante_path(current_path: &str, segment: &str) -> String {
    if current_path.is_empty() {
        format!("/{segment}")
    } else {
        format!("{current_path}/{segment}")
    }
}

fn display_ex_ante_path(path: &str) -> &str {
    if path.is_empty() { "/" } else { path }
}

pub fn parse_place_order_result(response: &Value) -> Result<PlaceOrderResult> {
    let order_data = response
        .get("placeOrder")
        .and_then(|v| v.get("orderData"))
        .ok_or_else(|| anyhow!("Trade response invalid: missing placeOrder.orderData"))?;

    let order_id = order_data
        .get("orderId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Trade response invalid: missing order id"))?
        .to_string();

    let is_marketable = order_data.get("isMarketable").and_then(Value::as_bool);

    Ok(PlaceOrderResult {
        order_id,
        is_marketable,
    })
}

pub fn parse_cancel_order_result(response: &Value) -> Result<()> {
    let cancel_order = response
        .get("cancelOrder")
        .ok_or_else(|| anyhow!("Trade response invalid: missing cancelOrder"))?;

    if cancel_order.is_null() {
        bail!("Trade response invalid: cancelOrder was null");
    }

    if !cancel_order.is_object() {
        bail!("Trade response invalid: cancelOrder must be an object");
    }

    Ok(())
}

pub fn trade_tradability_variables(
    person_id: &str,
    portfolio_id: &str,
    isin: &str,
) -> Result<Value> {
    Ok(json!({
        "personId": required_non_empty(person_id, "person_id")?,
        "portfolioId": required_non_empty(portfolio_id, "portfolio_id")?,
        "isin": required_non_empty(isin, "isin")?,
    }))
}

pub fn trade_appropriateness_result_variables(
    person_id: &str,
    portfolio_id: &str,
) -> Result<Value> {
    Ok(json!({
        "personId": required_non_empty(person_id, "person_id")?,
        "portfolioId": required_non_empty(portfolio_id, "portfolio_id")?,
    }))
}

pub fn trade_cancel_order_variables(portfolio_id: &str, order_id: &str) -> Result<Value> {
    Ok(json!({
        "portfolioId": required_non_empty(portfolio_id, "portfolio_id")?,
        "orderId": required_non_empty(order_id, "order_id")?,
    }))
}

fn normalize_warning_query_locale(locale: &str) -> Result<String> {
    Ok(required_non_empty(locale, "locale")?.replace('_', "-"))
}

pub fn trade_appropriateness_warning_variables(locale: &str) -> Result<Value> {
    Ok(json!({
        "locale": normalize_warning_query_locale(locale)?,
    }))
}

pub struct SingleExAnteFields<'a> {
    pub person_id: &'a str,
    pub portfolio_id: &'a str,
    pub isin: &'a str,
    pub side: TradeSide,
    pub estimated_order_volume: f64,
    pub number_of_shares: f64,
    pub venue: &'a str,
    pub is_whole_position_sold: bool,
}

pub fn trade_single_ex_ante_variables(fields: SingleExAnteFields<'_>) -> Result<Value> {
    Ok(json!({
        "personId": required_non_empty(fields.person_id, "person_id")?,
        "portfolioId": required_non_empty(fields.portfolio_id, "portfolio_id")?,
        "isin": required_non_empty(fields.isin, "isin")?,
        "side": fields.side.as_graphql(),
        "estimatedOrderVolume": round_estimated_order_volume_for_ex_ante(
            required_positive_decimal(fields.estimated_order_volume, "estimated_order_volume")?
        ),
        "numberOfShares": required_positive_decimal(fields.number_of_shares, "number_of_shares")?,
        "venue": required_non_empty(fields.venue, "venue")?,
        "isWholePositionSold": fields.is_whole_position_sold,
    }))
}

pub fn trade_fill_forecast_variables(
    portfolio_id: &str,
    isin: &str,
    currency: &str,
    venue: Option<&str>,
) -> Result<Value> {
    Ok(json!({
        "portfolioId": required_non_empty(portfolio_id, "portfolio_id")?,
        "input": {
            "isin": required_non_empty(isin, "isin")?,
            "side": "BUY",
            "currency": required_non_empty(currency, "currency")?,
            "venue": venue.map(|v| v.trim()).filter(|v| !v.is_empty()),
        }
    }))
}

pub struct PlaceOrderFields<'a> {
    pub side: TradeSide,
    pub portfolio_id: &'a str,
    pub isin: &'a str,
    pub number_of_shares: f64,
    pub currency: &'a str,
    pub venue: &'a str,
    pub limit_price: Option<f64>,
    pub stop_price: Option<f64>,
    pub appropriateness_id: Option<&'a str>,
    pub acknowledged_warning_version: Option<&'a str>,
    pub fill_forecast_id: Option<&'a str>,
    pub displayed_fill_probability: Option<f64>,
}

pub fn trade_place_order_variables(fields: PlaceOrderFields<'_>) -> Result<Value> {
    let mut input = json!({
        "isin": required_non_empty(fields.isin, "isin")?,
        "side": fields.side.as_graphql(),
        "numberOfShares": required_positive_decimal(fields.number_of_shares, "number_of_shares")?,
        "currency": required_non_empty(fields.currency, "currency")?,
        "venue": required_non_empty(fields.venue, "venue")?,
    });

    if let Some(limit_price) = fields.limit_price {
        input["limitPrice"] =
            serde_json::to_value(required_positive_decimal(limit_price, "limit_price")?)?;
    }

    if let Some(stop_price) = fields.stop_price {
        input["stopPrice"] =
            serde_json::to_value(required_positive_decimal(stop_price, "stop_price")?)?;
    }

    if let Some(appropriateness_id) = fields.appropriateness_id {
        input["appropriatenessId"] = Value::String(required_non_empty(
            appropriateness_id,
            "appropriateness_id",
        )?);
    }

    if let Some(version) = fields.acknowledged_warning_version {
        input["acknowledgedAppropriatenessWarningVersion"] =
            Value::String(required_non_empty(version, "acknowledged_warning_version")?);
    }

    if let (Some(fill_forecast_id), Some(probability)) =
        (fields.fill_forecast_id, fields.displayed_fill_probability)
    {
        input["fillForecastResult"] = json!({
            "fillForecastId": required_non_empty(fill_forecast_id, "fill_forecast_id")?,
            "displayedFillProbability": required_positive_decimal(
                probability,
                "displayed_fill_probability"
            )?,
        });
    }

    Ok(json!({
        "portfolioId": required_non_empty(fields.portfolio_id, "portfolio_id")?,
        "isBuy": fields.side.is_buy(),
        "input": input,
    }))
}

pub fn market_buy_shares_from_amount(
    amount: f64,
    price: f64,
    force_one_share_if_amount_positive: bool,
) -> u64 {
    if !amount.is_finite() || !price.is_finite() || amount <= 0.0 || price <= 0.0 {
        return 0;
    }

    let floored = (amount / price).floor();
    if floored >= 1.0 {
        if floored >= u64::MAX as f64 {
            return u64::MAX;
        }
        return floored as u64;
    }

    if force_one_share_if_amount_positive {
        1
    } else {
        0
    }
}

pub fn trade_side_quote_price(
    side: TradeSide,
    quote: &SecurityTick,
) -> Result<TradePriceSelection> {
    match side {
        TradeSide::Buy => Ok(TradePriceSelection {
            basis: "ask_price",
            price: quote.ask_price.ok_or_else(|| {
                anyhow!("EX_ANTE_COST_UNAVAILABLE: missing askPrice required for buy trade pricing")
            })?,
        }),
        TradeSide::Sell => Ok(TradePriceSelection {
            basis: "bid_price",
            price: quote.bid_price.ok_or_else(|| {
                anyhow!(
                    "EX_ANTE_COST_UNAVAILABLE: missing bidPrice required for sell trade pricing"
                )
            })?,
        }),
    }
}

pub fn trade_estimated_order_price(
    side: TradeSide,
    order_type: &str,
    limit_price: Option<f64>,
    stop_price: Option<f64>,
    quote: &SecurityTick,
) -> Result<TradePriceSelection> {
    let side_quote = trade_side_quote_price(side, quote)?;

    match order_type {
        "market" => Ok(side_quote),
        "limit" => {
            let limit_price = limit_price.ok_or_else(|| {
                anyhow!("Trade input invalid: limit order requires limit_price for estimate")
            })?;
            Ok(match side {
                TradeSide::Buy if limit_price < side_quote.price => TradePriceSelection {
                    basis: "limit_price",
                    price: limit_price,
                },
                TradeSide::Sell if limit_price > side_quote.price => TradePriceSelection {
                    basis: "limit_price",
                    price: limit_price,
                },
                _ => side_quote,
            })
        }
        "stop" => {
            let stop_price = stop_price.ok_or_else(|| {
                anyhow!("Trade input invalid: stop order requires stop_price for estimate")
            })?;
            Ok(match side {
                TradeSide::Buy if stop_price > side_quote.price => TradePriceSelection {
                    basis: "stop_price",
                    price: stop_price,
                },
                TradeSide::Sell if stop_price < side_quote.price => TradePriceSelection {
                    basis: "stop_price",
                    price: stop_price,
                },
                _ => side_quote,
            })
        }
        unsupported => bail!(
            "Trade input invalid: unsupported order type '{}' for estimate",
            unsupported
        ),
    }
}

pub fn round_estimated_order_volume_for_ex_ante(value: f64) -> f64 {
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphql_parser::query::parse_query;

    #[test]
    fn market_buy_shares_rounds_down() {
        assert_eq!(market_buy_shares_from_amount(100.0, 11.0, false), 9);
        assert_eq!(market_buy_shares_from_amount(20.0, 21.0, false), 0);
    }

    #[test]
    fn market_buy_shares_supports_partial_buy_limit_fallback() {
        assert_eq!(market_buy_shares_from_amount(5.0, 11.0, true), 1);
    }

    #[test]
    fn market_buy_shares_rejects_invalid_inputs() {
        assert_eq!(market_buy_shares_from_amount(0.0, 11.0, true), 0);
        assert_eq!(market_buy_shares_from_amount(11.0, 0.0, true), 0);
        assert_eq!(market_buy_shares_from_amount(f64::NAN, 11.0, true), 0);
    }

    #[test]
    fn trade_side_quote_price_uses_ask_for_buy_and_bid_for_sell() {
        let quote = SecurityTick {
            ask_price: Some(11.0),
            bid_price: Some(10.0),
            mid_price: 10.5,
            currency: "EUR".to_string(),
            is_outdated: false,
            timestamp_utc: None,
        };

        assert_eq!(
            trade_side_quote_price(TradeSide::Buy, &quote).expect("buy quote"),
            TradePriceSelection {
                basis: "ask_price",
                price: 11.0,
            }
        );
        assert_eq!(
            trade_side_quote_price(TradeSide::Sell, &quote).expect("sell quote"),
            TradePriceSelection {
                basis: "bid_price",
                price: 10.0,
            }
        );
    }

    #[test]
    fn trade_side_quote_price_fails_closed_when_required_quote_leg_is_missing() {
        let quote = SecurityTick {
            ask_price: None,
            bid_price: None,
            mid_price: 10.5,
            currency: "EUR".to_string(),
            is_outdated: false,
            timestamp_utc: None,
        };

        assert!(
            trade_side_quote_price(TradeSide::Buy, &quote)
                .expect_err("buy should fail")
                .to_string()
                .contains("missing askPrice")
        );
        assert!(
            trade_side_quote_price(TradeSide::Sell, &quote)
                .expect_err("sell should fail")
                .to_string()
                .contains("missing bidPrice")
        );
    }

    #[test]
    fn trade_estimated_order_price_matches_ios_limit_and_stop_rules() {
        let quote = SecurityTick {
            ask_price: Some(11.0),
            bid_price: Some(10.0),
            mid_price: 10.5,
            currency: "EUR".to_string(),
            is_outdated: false,
            timestamp_utc: None,
        };

        assert_eq!(
            trade_estimated_order_price(TradeSide::Buy, "market", None, None, &quote)
                .expect("buy market"),
            TradePriceSelection {
                basis: "ask_price",
                price: 11.0,
            }
        );
        assert_eq!(
            trade_estimated_order_price(TradeSide::Buy, "limit", Some(9.5), None, &quote)
                .expect("buy limit"),
            TradePriceSelection {
                basis: "limit_price",
                price: 9.5,
            }
        );
        assert_eq!(
            trade_estimated_order_price(TradeSide::Buy, "limit", Some(12.0), None, &quote)
                .expect("buy high limit"),
            TradePriceSelection {
                basis: "ask_price",
                price: 11.0,
            }
        );
        assert_eq!(
            trade_estimated_order_price(TradeSide::Buy, "stop", None, Some(12.5), &quote)
                .expect("buy stop"),
            TradePriceSelection {
                basis: "stop_price",
                price: 12.5,
            }
        );
        assert_eq!(
            trade_estimated_order_price(TradeSide::Sell, "market", None, None, &quote)
                .expect("sell market"),
            TradePriceSelection {
                basis: "bid_price",
                price: 10.0,
            }
        );
        assert_eq!(
            trade_estimated_order_price(TradeSide::Sell, "limit", Some(12.0), None, &quote)
                .expect("sell limit"),
            TradePriceSelection {
                basis: "limit_price",
                price: 12.0,
            }
        );
        assert_eq!(
            trade_estimated_order_price(TradeSide::Sell, "stop", None, Some(9.0), &quote)
                .expect("sell stop"),
            TradePriceSelection {
                basis: "stop_price",
                price: 9.0,
            }
        );
    }

    #[test]
    fn ex_ante_order_volume_rounding_uses_4dp_half_up_for_positive_values() {
        assert_eq!(
            round_estimated_order_volume_for_ex_ante(123.45674),
            123.4567
        );
        assert_eq!(
            round_estimated_order_volume_for_ex_ante(123.45675),
            123.4568
        );
    }

    #[test]
    fn ex_ante_order_volume_rounding_returns_zero_for_invalid_values() {
        assert_eq!(round_estimated_order_volume_for_ex_ante(0.0), 0.0);
        assert_eq!(round_estimated_order_volume_for_ex_ante(-1.0), 0.0);
        assert_eq!(round_estimated_order_volume_for_ex_ante(f64::INFINITY), 0.0);
    }

    #[test]
    fn trade_variable_builders_validate_inputs() {
        let vars = trade_tradability_variables("person-1", "portfolio-1", "US0378331005")
            .expect("tradability vars");
        assert_eq!(vars["personId"], "person-1");
        assert_eq!(vars["portfolioId"], "portfolio-1");
        assert_eq!(vars["isin"], "US0378331005");

        let warning_vars = trade_appropriateness_warning_variables("en_DE").expect("warning vars");
        assert_eq!(warning_vars["locale"], "en-DE");

        let warning_vars = trade_appropriateness_warning_variables("en-DE").expect("warning vars");
        assert_eq!(warning_vars["locale"], "en-DE");

        let err = trade_appropriateness_warning_variables(" ").unwrap_err();
        assert!(err.to_string().contains("locale"));
    }

    #[test]
    fn trade_cancel_order_variables_map_order_id() {
        let vars = trade_cancel_order_variables("portfolio-1", " order-1 ").expect("cancel vars");
        assert_eq!(vars["portfolioId"], "portfolio-1");
        assert_eq!(vars["orderId"], "order-1");
    }

    #[test]
    fn trade_cancel_order_variables_reject_blank_order_id() {
        let err = trade_cancel_order_variables("portfolio-1", " ").expect_err("blank order id");
        assert!(err.to_string().contains("order_id"));
    }

    #[test]
    fn trade_place_order_variables_include_optional_compliance_fields() {
        let vars = trade_place_order_variables(PlaceOrderFields {
            side: TradeSide::Buy,
            portfolio_id: "portfolio-1",
            isin: "US0378331005",
            number_of_shares: 9.0,
            currency: "EUR",
            venue: "MUNC",
            limit_price: Some(123.45),
            stop_price: None,
            appropriateness_id: Some("app-1"),
            acknowledged_warning_version: Some("v1"),
            fill_forecast_id: Some("ff-1"),
            displayed_fill_probability: Some(0.92),
        })
        .expect("place order vars");

        assert_eq!(vars["portfolioId"], "portfolio-1");
        assert_eq!(vars["isBuy"], true);
        assert_eq!(vars["input"]["limitPrice"], 123.45);
        assert_eq!(vars["input"]["stopPrice"], Value::Null);
        assert_eq!(vars["input"]["appropriatenessId"], "app-1");
        assert_eq!(
            vars["input"]["acknowledgedAppropriatenessWarningVersion"],
            "v1"
        );
        assert_eq!(
            vars["input"]["fillForecastResult"]["fillForecastId"],
            "ff-1"
        );
        assert_eq!(
            vars["input"]["fillForecastResult"]["displayedFillProbability"],
            0.92
        );
    }

    #[test]
    fn trade_place_order_variables_include_stop_price_without_limit_price() {
        let vars = trade_place_order_variables(PlaceOrderFields {
            side: TradeSide::Buy,
            portfolio_id: "portfolio-1",
            isin: "US0378331005",
            number_of_shares: 2.0,
            currency: "EUR",
            venue: "MUNC",
            limit_price: None,
            stop_price: Some(88.10),
            appropriateness_id: None,
            acknowledged_warning_version: None,
            fill_forecast_id: None,
            displayed_fill_probability: None,
        })
        .expect("place order vars");

        assert_eq!(vars["portfolioId"], "portfolio-1");
        assert_eq!(vars["isBuy"], true);
        assert_eq!(vars["input"]["limitPrice"], Value::Null);
        assert_eq!(vars["input"]["stopPrice"], 88.10);
        assert_eq!(vars["input"]["fillForecastResult"], Value::Null);
    }

    #[test]
    fn trade_place_order_variables_support_sell_side() {
        let vars = trade_place_order_variables(PlaceOrderFields {
            side: TradeSide::Sell,
            portfolio_id: "portfolio-1",
            isin: "US0378331005",
            number_of_shares: 2.0,
            currency: "EUR",
            venue: "MUNC",
            limit_price: None,
            stop_price: None,
            appropriateness_id: None,
            acknowledged_warning_version: None,
            fill_forecast_id: None,
            displayed_fill_probability: None,
        })
        .expect("place order vars");

        assert_eq!(vars["isBuy"], false);
        assert_eq!(vars["input"]["side"], "SELL");
        assert_eq!(vars["input"]["numberOfShares"], 2.0);
    }

    #[test]
    fn trade_query_documents_parse_as_graphql_documents() {
        let queries = [
            TRADE_TRADABILITY_QUERY,
            TRADE_APPROPRIATENESS_RESULT_QUERY,
            TRADE_APPROPRIATENESS_WARNING_QUERY,
            TRADE_SECURITY_TICK_QUERY,
            TRADE_SINGLE_EX_ANTE_COSTS_QUERY,
            TRADE_FILL_FORECAST_MUTATION,
            TRADE_PLACE_ORDER_MUTATION,
            TRADE_CANCEL_ORDER_MUTATION,
        ];

        for query in queries {
            parse_query::<String>(query).expect("query should parse as valid GraphQL");
        }
    }

    #[test]
    fn parse_tradability_gate_extracts_buy_venue_and_status() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "buyTradabilityForTrading": {
                            "tradabilityStatus": "TRADABLE_WITH_APPROPRIATENESS",
                            "venues": [
                                {
                                    "venue": "MUNC",
                                    "tradabilityStatus": "TRADABLE_WITH_APPROPRIATENESS",
                                    "unavailabilityReason": null
                                }
                            ],
                            "primaryVenue": {
                                "venue": "MUNC",
                                "status": "OPEN"
                            }
                        }
                    }
                }
            }
        });

        let gate = parse_tradability_gate(&response, TradeSide::Buy, None)
            .expect("tradability gate should parse");
        assert_eq!(gate.status, "TRADABLE_WITH_APPROPRIATENESS");
        assert_eq!(gate.selected_venue, "MUNC");
        assert!(gate.requires_appropriateness);
        assert_eq!(gate.selected_venue_sellable, None);
    }

    #[test]
    fn parse_tradability_gate_respects_buy_venue_override() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "buyTradabilityForTrading": {
                            "tradabilityStatus": "TRADABLE_WITHOUT_APPROPRIATENESS",
                            "venues": [
                                {
                                    "venue": "MUNC",
                                    "tradabilityStatus": "TRADABLE_WITHOUT_APPROPRIATENESS",
                                    "unavailabilityReason": null
                                },
                                {
                                    "venue": "XETR",
                                    "tradabilityStatus": "TRADABLE_WITHOUT_APPROPRIATENESS",
                                    "unavailabilityReason": null
                                }
                            ],
                            "primaryVenue": {
                                "venue": "MUNC",
                                "status": "OPEN"
                            }
                        }
                    }
                }
            }
        });

        let gate = parse_tradability_gate(&response, TradeSide::Buy, Some("XETR"))
            .expect("tradability gate should parse");
        assert_eq!(gate.selected_venue, "XETR");
        assert!(!gate.requires_appropriateness);
    }

    #[test]
    fn parse_tradability_gate_extracts_sellable_for_sell_side() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "sellTradabilityForTrading": {
                            "tradabilityStatus": "TRADABLE_WITHOUT_APPROPRIATENESS",
                            "venues": [
                                {
                                    "venue": "MUNC",
                                    "tradabilityStatus": "TRADABLE_WITHOUT_APPROPRIATENESS",
                                    "unavailabilityReason": null
                                }
                            ],
                            "primaryVenue": {
                                "venue": "MUNC",
                                "status": "OPEN"
                            }
                        },
                        "inventory": {
                            "position": {
                                "sellableByVenue": [
                                    {"venue": "MUNC", "sellable": 2.5}
                                ]
                            }
                        }
                    }
                }
            }
        });

        let gate = parse_tradability_gate(&response, TradeSide::Sell, None)
            .expect("sell tradability gate should parse");
        assert_eq!(gate.selected_venue_sellable, Some(2.5));
    }

    #[test]
    fn parse_tradability_gate_errors_when_selected_sell_venue_has_no_sellable_entry() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "sellTradabilityForTrading": {
                            "tradabilityStatus": "TRADABLE_WITHOUT_APPROPRIATENESS",
                            "venues": [
                                {
                                    "venue": "MUNC",
                                    "tradabilityStatus": "TRADABLE_WITHOUT_APPROPRIATENESS",
                                    "unavailabilityReason": null
                                }
                            ],
                            "primaryVenue": {
                                "venue": "MUNC",
                                "status": "OPEN"
                            }
                        },
                        "inventory": {
                            "position": {
                                "sellableByVenue": [
                                    {"venue": "XETR", "sellable": 2.5}
                                ]
                            }
                        }
                    }
                }
            }
        });

        let err = parse_tradability_gate(&response, TradeSide::Sell, None)
            .expect_err("missing selected-venue sellable should fail");
        assert!(
            err.to_string()
                .contains("missing sellable quantity for selected venue 'MUNC'")
        );
    }

    #[test]
    fn trade_single_ex_ante_variables_support_sell_side_and_whole_position_flag() {
        let vars = trade_single_ex_ante_variables(SingleExAnteFields {
            person_id: "person-1",
            portfolio_id: "portfolio-1",
            isin: "US0378331005",
            side: TradeSide::Sell,
            estimated_order_volume: 123.45678,
            number_of_shares: 2.5,
            venue: "MUNC",
            is_whole_position_sold: true,
        })
        .expect("ex-ante vars");

        assert_eq!(vars["personId"], "person-1");
        assert_eq!(vars["portfolioId"], "portfolio-1");
        assert_eq!(vars["side"], "SELL");
        assert_eq!(vars["venue"], "MUNC");
        assert_eq!(vars["numberOfShares"], 2.5);
        assert_eq!(vars["estimatedOrderVolume"], 123.4568);
        assert_eq!(vars["isWholePositionSold"], true);
    }

    #[test]
    fn evaluate_appropriateness_gate_flags_not_evaluated() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "appropriatenessInfo": {
                        "appropriatenessId": null,
                        "result": "NOT_EVALUATED"
                    }
                }
            }
        });

        let gate = evaluate_appropriateness_gate(&response, true)
            .expect("appropriateness gate should parse");
        assert_eq!(gate.status, "NOT_EVALUATED");
        assert!(!gate.passed);
    }

    #[test]
    fn evaluate_suitability_is_fail_closed_for_unknown_statuses() {
        let review = evaluate_suitability(Some("BACKEND_NEW_STATUS"));

        assert_eq!(review.status, "BACKEND_NEW_STATUS");
        assert!(!review.is_suitable);
        assert!(review.requires_accept_unsuitable);
    }

    #[test]
    fn parse_security_tick_extracts_quote_info() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "quoteTick": {
                            "askPrice": 123.46,
                            "bidPrice": 123.44,
                            "midPrice": 123.45,
                            "currency": "EUR",
                            "isOutdated": false,
                            "timestampUtc": {"time": "2026-03-04T10:15:30Z"}
                        }
                    }
                }
            }
        });

        let quote = parse_security_tick(&response).expect("quote tick should parse");
        assert_eq!(quote.currency, "EUR");
        assert_eq!(quote.ask_price, Some(123.46));
        assert_eq!(quote.bid_price, Some(123.44));
        assert_eq!(quote.mid_price, 123.45);
        assert!(!quote.is_outdated);
    }

    #[test]
    fn parse_security_tick_treats_zero_optional_quote_legs_as_absent() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "quoteTick": {
                            "askPrice": 0,
                            "bidPrice": 0.0,
                            "midPrice": 123.45,
                            "currency": "EUR",
                            "isOutdated": false,
                            "timestampUtc": {"time": "2026-03-04T10:15:30Z"}
                        }
                    }
                }
            }
        });

        let quote = parse_security_tick(&response).expect("quote tick should parse");
        assert_eq!(quote.ask_price, None);
        assert_eq!(quote.bid_price, None);
        assert_eq!(quote.mid_price, 123.45);
    }

    #[test]
    fn parse_security_tick_rejects_negative_optional_quote_legs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "quoteTick": {
                            "askPrice": -1.0,
                            "bidPrice": 123.44,
                            "midPrice": 123.45,
                            "currency": "EUR",
                            "isOutdated": false,
                            "timestampUtc": {"time": "2026-03-04T10:15:30Z"}
                        }
                    }
                }
            }
        });

        let err =
            parse_security_tick(&response).expect_err("negative optional quote leg should fail");
        assert_eq!(
            err.to_string(),
            "Trade response invalid: invalid security quote askPrice '-1'"
        );
    }

    #[test]
    fn parse_security_issuer_document_links_prefers_locale_specific_secondary() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "issuerLinks": {
                            "kidLinks": [
                                {
                                    "isPrimary": true,
                                    "url": "https://example.test/primary-kid.pdf",
                                    "locale": "de_DE"
                                },
                                {
                                    "isPrimary": false,
                                    "url": "https://example.test/en-kid.pdf",
                                    "locale": "en_DE"
                                }
                            ]
                        }
                    }
                }
            }
        });

        let links = parse_security_issuer_document_links(&response, "en_DE")
            .expect("issuer links should parse");
        assert_eq!(
            links.primary_kid_url.as_deref(),
            Some("https://example.test/primary-kid.pdf")
        );
        assert_eq!(
            links.secondary_kid_url.as_deref(),
            Some("https://example.test/en-kid.pdf")
        );
    }

    #[test]
    fn parse_security_issuer_document_links_returns_none_when_missing() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "issuerLinks": {
                            "kidLinks": []
                        }
                    }
                }
            }
        });

        let links = parse_security_issuer_document_links(&response, "en_DE")
            .expect("issuer links should parse");
        assert_eq!(links.primary_kid_url, None);
        assert_eq!(links.secondary_kid_url, None);
    }

    #[test]
    fn parse_security_issuer_document_links_skips_blank_urls() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "security": {
                        "issuerLinks": {
                            "kidLinks": [
                                {
                                    "isPrimary": true,
                                    "url": "",
                                    "locale": "de_DE"
                                },
                                {
                                    "isPrimary": true,
                                    "url": "https://example.test/primary-kid.pdf",
                                    "locale": "de_DE"
                                },
                                {
                                    "isPrimary": false,
                                    "url": "   ",
                                    "locale": "en_DE"
                                },
                                {
                                    "isPrimary": false,
                                    "url": "https://example.test/en-kid.pdf",
                                    "locale": "en_DE"
                                }
                            ]
                        }
                    }
                }
            }
        });

        let links = parse_security_issuer_document_links(&response, "en_DE")
            .expect("issuer links should parse");
        assert_eq!(
            links.primary_kid_url.as_deref(),
            Some("https://example.test/primary-kid.pdf")
        );
        assert_eq!(
            links.secondary_kid_url.as_deref(),
            Some("https://example.test/en-kid.pdf")
        );
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_backfills_missing_breakdown_fields() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "entryCosts": {
                            "total": {
                                "amount": 1.23,
                                "percentage": 0.02
                            }
                        },
                        "effectOnReturn": {
                            "initialYearCosts": Value::Null,
                            "followingYearsCosts": Value::Null,
                            "finalYearCosts": {}
                        }
                    }
                }
            }
        });

        let costs = extract_single_trade_ex_ante_costs(&response).expect("ex-ante should parse");
        assert_eq!(costs["entryCosts"]["total"]["amount"], 1.23);
        assert_eq!(costs["entryCosts"]["serviceCosts"]["amount"], 0);
        assert_eq!(costs["effectOnReturn"]["finalYearCosts"]["percentage"], 0);
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_fills_missing_required_breakdown_fields_with_zero() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "id": "cost-id",
                        "entryCosts": {
                            "productCosts": {
                                "amount": 0.34,
                                "percentage": 0.00075
                            },
                            "total": {
                                "amount": 0.34,
                                "percentage": 0.00075
                            }
                        },
                        "ongoingCosts": {
                            "serviceCosts": {
                                "percentage": 0.00011
                            }
                        },
                        "exitCosts": {},
                        "effectOnReturn": {
                            "initialYearCosts": {
                                "amount": 0.46
                            },
                            "followingYearsCosts": {},
                            "finalYearCosts": {}
                        }
                    }
                }
            }
        });

        let costs = extract_single_trade_ex_ante_costs(&response).expect("ex-ante should parse");

        assert_eq!(costs["entryCosts"]["serviceCosts"]["amount"], 0);
        assert_eq!(costs["entryCosts"]["serviceCosts"]["percentage"], 0);
        assert_eq!(costs["ongoingCosts"]["serviceCosts"]["amount"], 0);
        assert_eq!(costs["exitCosts"]["total"]["amount"], 0);
        assert_eq!(costs["effectOnReturn"]["initialYearCosts"]["percentage"], 0);
        assert_eq!(costs["effectOnReturn"]["finalYearCosts"]["amount"], 0);
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_allows_null_entry_costs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "entryCosts": Value::Null,
                        "effectOnReturn": {
                            "initialYearCosts": Value::Null,
                            "followingYearsCosts": Value::Null,
                            "finalYearCosts": {}
                        }
                    }
                }
            }
        });

        let costs = extract_single_trade_ex_ante_costs(&response).expect("ex-ante should parse");
        assert_eq!(costs.get("entryCosts"), Some(&Value::Null));
        assert_eq!(costs["ongoingCosts"]["total"]["amount"], 0);
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_rejects_missing_entry_costs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "ongoingCosts": {
                            "total": {
                                "amount": 0.12,
                                "percentage": 0.00026
                            }
                        },
                        "exitCosts": {},
                        "effectOnReturn": {
                            "initialYearCosts": Value::Null,
                            "followingYearsCosts": Value::Null,
                            "finalYearCosts": {}
                        }
                    }
                }
            }
        });

        let err = extract_single_trade_ex_ante_costs(&response).expect_err("ex-ante should fail");
        assert!(err.to_string().contains(
            "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '/entryCosts'"
        ));
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_rejects_invalid_non_null_entry_costs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "entryCosts": 0,
                        "effectOnReturn": {
                            "initialYearCosts": Value::Null,
                            "followingYearsCosts": Value::Null,
                            "finalYearCosts": {}
                        }
                    }
                }
            }
        });

        let err = extract_single_trade_ex_ante_costs(&response).expect_err("ex-ante should fail");
        assert!(err.to_string().contains(
            "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '/entryCosts'"
        ));
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_allows_null_initial_year_costs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "entryCosts": Value::Null,
                        "effectOnReturn": {
                            "initialYearCosts": Value::Null,
                            "followingYearsCosts": {
                                "amount": 0.12,
                                "percentage": 0.00026
                            },
                            "finalYearCosts": {
                                "amount": 0.17,
                                "percentage": 0.00037
                            }
                        }
                    }
                }
            }
        });

        let costs = extract_single_trade_ex_ante_costs(&response).expect("ex-ante should parse");
        assert_eq!(costs["effectOnReturn"]["initialYearCosts"], Value::Null);
        assert_eq!(
            costs["effectOnReturn"]["followingYearsCosts"]["amount"],
            0.12
        );
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_allows_null_following_years_costs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "entryCosts": Value::Null,
                        "effectOnReturn": {
                            "initialYearCosts": {
                                "amount": 0.46,
                                "percentage": 0.00101
                            },
                            "followingYearsCosts": Value::Null,
                            "finalYearCosts": {
                                "amount": 0.17,
                                "percentage": 0.00037
                            }
                        }
                    }
                }
            }
        });

        let costs = extract_single_trade_ex_ante_costs(&response).expect("ex-ante should parse");
        assert_eq!(costs["effectOnReturn"]["initialYearCosts"]["amount"], 0.46);
        assert_eq!(costs["effectOnReturn"]["followingYearsCosts"], Value::Null);
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_rejects_missing_initial_year_costs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "entryCosts": Value::Null,
                        "effectOnReturn": {
                            "followingYearsCosts": Value::Null,
                            "finalYearCosts": {}
                        }
                    }
                }
            }
        });

        let err = extract_single_trade_ex_ante_costs(&response).expect_err("ex-ante should fail");
        assert!(err.to_string().contains(
            "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '/effectOnReturn/initialYearCosts'"
        ));
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_rejects_missing_following_years_costs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "entryCosts": Value::Null,
                        "effectOnReturn": {
                            "initialYearCosts": Value::Null,
                            "finalYearCosts": {}
                        }
                    }
                }
            }
        });

        let err = extract_single_trade_ex_ante_costs(&response).expect_err("ex-ante should fail");
        assert!(err.to_string().contains(
            "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '/effectOnReturn/followingYearsCosts'"
        ));
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_rejects_invalid_non_null_initial_year_costs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "entryCosts": Value::Null,
                        "effectOnReturn": {
                            "initialYearCosts": 0,
                            "followingYearsCosts": Value::Null,
                            "finalYearCosts": {}
                        }
                    }
                }
            }
        });

        let err = extract_single_trade_ex_ante_costs(&response).expect_err("ex-ante should fail");
        assert!(err.to_string().contains(
            "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '/effectOnReturn/initialYearCosts'"
        ));
    }

    #[test]
    fn extract_single_trade_ex_ante_costs_rejects_invalid_non_null_following_years_costs() {
        let response = json!({
            "account": {
                "brokerPortfolio": {
                    "singleTradeExAnteCosts": {
                        "entryCosts": Value::Null,
                        "effectOnReturn": {
                            "initialYearCosts": Value::Null,
                            "followingYearsCosts": 0,
                            "finalYearCosts": {}
                        }
                    }
                }
            }
        });

        let err = extract_single_trade_ex_ante_costs(&response).expect_err("ex-ante should fail");
        assert!(err.to_string().contains(
            "EX_ANTE_COST_UNAVAILABLE: invalid singleTradeExAnteCosts payload at '/effectOnReturn/followingYearsCosts'"
        ));
    }

    #[test]
    fn parse_place_order_result_extracts_order_id_and_marketable_flag() {
        let response = json!({
            "placeOrder": {
                "orderData": {
                    "orderId": "order-1",
                    "isMarketable": true
                }
            }
        });

        let order = parse_place_order_result(&response).expect("order result should parse");
        assert_eq!(order.order_id, "order-1");
        assert_eq!(order.is_marketable, Some(true));
    }

    #[test]
    fn parse_cancel_order_result_accepts_present_object() {
        let response = json!({
            "cancelOrder": {
                "id": "portfolio-1"
            }
        });

        parse_cancel_order_result(&response).expect("cancel result should parse");
    }

    #[test]
    fn parse_cancel_order_result_rejects_missing_object() {
        let response = json!({});
        let err = parse_cancel_order_result(&response).expect_err("missing object should fail");
        assert!(err.to_string().contains("missing cancelOrder"));
    }

    #[test]
    fn parse_cancel_order_result_rejects_null_object() {
        let response = json!({
            "cancelOrder": Value::Null
        });
        let err = parse_cancel_order_result(&response).expect_err("null object should fail");
        assert!(err.to_string().contains("cancelOrder was null"));
    }

    #[test]
    fn parse_cancel_order_result_rejects_non_object_payload() {
        let response = json!({
            "cancelOrder": "portfolio-1"
        });
        let err = parse_cancel_order_result(&response).expect_err("non-object payload should fail");
        assert!(err.to_string().contains("cancelOrder must be an object"));
    }
}
