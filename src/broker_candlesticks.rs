use anyhow::{Result, anyhow};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};

use crate::active_session::load_active_session;
use crate::broker_queries::normalize_broker_isin;
use crate::cli::{BrokerCandlesticksArgs, BrokerCandlesticksTimeframe};
use crate::config::AppConfig;
use crate::graphql::execute_graphql_typed;
use crate::quote_source::QuoteSource;
use crate::session::SessionManager;
use crate::session_refresh::execute_with_refresh_retry;
use crate::trade_execution::normalize_decimal_str;

pub(crate) const QUERY: &str = r#"
query BrokerCandlesticks($isin: String!, $source: MarketDataSource!, $timeFrames: [TimeFrame!]!) {
  candlesticksTimeSeries(isin: $isin, source: $source, timeFrames: $timeFrames) {
    isin
    currency
    source
    candlesticks {
      timeFrame
      dataPoints {
        open
        high
        low
        close
        startTimestamp { time }
        endTimestamp { time }
      }
    }
  }
}
"#;

pub(crate) fn variables(
    isin: &str,
    timeframe: BrokerCandlesticksTimeframe,
    source: QuoteSource,
) -> Result<Value> {
    Ok(json!({
        "isin": normalize_broker_isin(isin, "isin")?,
        "timeFrames": [timeframe.as_graphql()],
        "source": source.as_graphql(),
    }))
}

pub(crate) fn execute(
    args: BrokerCandlesticksArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    // Validate before accessing the session or performing any network request.
    let variables = variables(&args.isin, args.timeframe, args.quote_source)?;
    let requested_isin = variables["isin"].as_str().expect("validated ISIN");
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let env = crate::resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, &dpop_options)?;
    let mut session = loaded.session;
    let response: Response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        &dpop_options,
        |token| {
            execute_graphql_typed(
                &env_cfg.graphql_url,
                token,
                QUERY,
                &variables,
                Some("BrokerCandlesticks"),
                loaded.access_context,
                &dpop_options,
            )
            .map_err(|error| {
                if error.downcast_ref::<serde_json::Error>().is_some() {
                    invalid("response could not be mapped")
                } else {
                    error
                }
            })
        },
    )?;
    map_candlesticks_response(requested_isin, args.timeframe, args.quote_source, response)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    candlesticks_time_series: SecuritySeries,
}

#[derive(Deserialize)]
struct SecuritySeries {
    isin: String,
    currency: String,
    source: String,
    candlesticks: Vec<Series>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Series {
    time_frame: String,
    data_points: Vec<Point>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Point {
    open: Box<RawValue>,
    high: Box<RawValue>,
    low: Box<RawValue>,
    close: Box<RawValue>,
    start_timestamp: Timestamp,
    end_timestamp: Timestamp,
}

#[derive(Deserialize)]
struct Timestamp {
    time: String,
}

fn invalid(message: &str) -> anyhow::Error {
    anyhow!("Broker response invalid: candlesticks {message}")
}

fn map_candlesticks_response(
    isin: &str,
    timeframe: BrokerCandlesticksTimeframe,
    source: QuoteSource,
    response: Response,
) -> Result<Value> {
    let security = response.candlesticks_time_series;
    if security.isin.trim() != isin {
        return Err(invalid("returned a mismatched ISIN"));
    }
    if security.source != source.as_graphql() {
        return Err(invalid("returned a mismatched source"));
    }
    let mut matching = security
        .candlesticks
        .iter()
        .filter(|series| series.time_frame.trim() == timeframe.as_graphql());
    let series = matching
        .next()
        .ok_or_else(|| invalid("requested timeframe is missing"))?;
    if matching.next().is_some() {
        return Err(invalid("requested timeframe is duplicated"));
    }
    if security.currency.len() != 3 || !security.currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(invalid("currency must be a three-letter currency code"));
    }
    let candles = series.data_points.iter().map(|point| {
        let start = DateTime::parse_from_rfc3339(&point.start_timestamp.time)
            .map_err(|_| invalid("contains an invalid start timestamp"))?;
        let end = DateTime::parse_from_rfc3339(&point.end_timestamp.time)
            .map_err(|_| invalid("contains an invalid end timestamp"))?;
        if start > end {
            return Err(invalid("interval start is after its end"));
        }
        Ok(json!({
            "open": price(&point.open)?,
            "high": price(&point.high)?,
            "low": price(&point.low)?,
            "close": price(&point.close)?,
            "start_time_utc": start.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true),
            "end_time_utc": end.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true),
        }))
    }).collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "isin": isin,
        "timeframe": timeframe.as_cli_value(),
        "source": source.as_cli_value(),
        "currency": security.currency,
        "candlesticks": candles,
    }))
}

// Keep decimal strings exact, including prices below the precision of f64.
// JSON numbers may use exponent notation; string prices must use plain-decimal notation.
fn price(value: &RawValue) -> Result<String> {
    let token = value.get();
    if token.len() > 4096 {
        return Err(invalid("price exceeds supported decimal size"));
    }
    let decoded;
    let raw = if token.starts_with('"') {
        decoded = serde_json::from_str::<String>(token)?;
        decoded.trim()
    } else {
        return numeric_price(token).ok_or_else(|| invalid("contains an invalid price"));
    };
    let unsigned = raw.strip_prefix('-').unwrap_or(raw);
    let mut parts = unsigned.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || whole.len() + fraction.len() == 0
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(invalid("contains an invalid price"));
    }
    Ok(normalize_decimal_str(raw))
}

fn numeric_price(raw: &str) -> Option<String> {
    let (mantissa, exponent) = match raw.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
        None => (raw, 0),
    };
    if !(-4096..=4096).contains(&exponent) {
        return None;
    }
    let negative = mantissa.starts_with('-');
    let unsigned = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let position = whole.len() as i64 + i64::from(exponent);
    let expanded = if position <= 0 {
        format!("0.{}{digits}", "0".repeat((-position) as usize))
    } else if position >= digits.len() as i64 {
        format!("{digits}{}", "0".repeat(position as usize - digits.len()))
    } else {
        let position = position as usize;
        format!("{}.{}", &digits[..position], &digits[position..])
    };
    Some(normalize_decimal_str(&if negative {
        format!("-{expanded}")
    } else {
        expanded
    }))
}

#[cfg(test)]
mod tests;
