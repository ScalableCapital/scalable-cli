use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::broker_queries::{
    BrokerInput, NormalizedBrokerDerivativesSearchQueryInput, timestamp_value_or_null,
};

pub fn project_broker_overview_response(input: &BrokerInput, response: &Value) -> Result<Value> {
    let valuation = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("valuation"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.valuation")
        })?;

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "valuation": {
            "total": valuation.get("valuation").cloned().unwrap_or(Value::Null),
            "securities": valuation.get("securitiesValuation").cloned().unwrap_or(Value::Null),
            "crypto": valuation.get("cryptoValuation").cloned().unwrap_or(Value::Null),
        },
        "timestamps": {
            "valuation_timestamp_utc": timestamp_value_or_null(
                valuation
                    .get("timestampUtc")
                    .or_else(|| valuation.get("timestamp")),
            ),
            "inventory_timestamp_utc": timestamp_value_or_null(
                valuation
                    .get("lastInventoryUpdateTimestampUtc")
                    .or_else(|| valuation.get("lastInventoryUpdateTimestamp")),
            ),
        },
        "performance": valuation
            .get("timeWeightedReturnByTimeframe")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    }))
}

pub fn project_broker_analytics_response(input: &BrokerInput, response: &Value) -> Result<Value> {
    let analysis = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("portfolioAnalysis"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.portfolioAnalysis")
        })?;
    let result = analysis.get("result").ok_or_else(|| {
        anyhow!("Broker response invalid: missing account.brokerPortfolio.portfolioAnalysis.result")
    })?;

    let invalid_securities = analysis
        .get("invalidSecurities")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(map_security_ref).collect::<Vec<_>>())
        .unwrap_or_default();

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "analysis_type": analysis.get("type").cloned().unwrap_or(Value::Null),
        "portfolio_coverage": analysis.get("portfolioCoverage").cloned().unwrap_or(Value::Null),
        "invalid_securities_count": invalid_securities.len(),
        "invalid_securities": invalid_securities,
        "result_id": result.get("id").cloned().unwrap_or(Value::Null),
        "last_updated_utc": timestamp_value_or_null(result.get("lastUpdated")),
        "health_checks": result
            .get("healthChecks")
            .and_then(|v| v.get("items"))
            .and_then(Value::as_array)
            .map(|items| items.iter().map(map_health_check).collect::<Vec<_>>())
            .unwrap_or_default(),
        "scenarios": result
            .get("scenarios")
            .and_then(|v| v.get("items"))
            .and_then(Value::as_array)
            .map(|items| items.iter().map(map_portfolio_scenario).collect::<Vec<_>>())
            .unwrap_or_default(),
        "allocations": result
            .get("allocations")
            .and_then(|v| v.get("items"))
            .and_then(Value::as_array)
            .map(|items| items.iter().map(map_portfolio_allocation).collect::<Vec<_>>())
            .unwrap_or_default(),
        "equity_company_styles": map_equity_company_styles(result.get("equityCompanyStyles")),
        "fixed_income_ratings": map_fixed_income_ratings(result.get("fixedIncomeRatings")),
        "payments": map_payments(result.get("payments")),
        "trial_period": map_trial_period(result.get("trialPeriod")),
    }))
}

pub fn project_broker_holdings_response(input: &BrokerInput, response: &Value) -> Result<Value> {
    let items = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("inventory"))
        .and_then(|v| v.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.inventory.items")
        })?;

    let mut projected = items
        .iter()
        .map(|item| {
            let position = item
                .get("inventory")
                .and_then(|v| v.get("position"))
                .cloned()
                .unwrap_or(Value::Null);
            let quote = item.get("quoteTick").cloned().unwrap_or(Value::Null);
            let performance = item
                .get("portfolioIsinPerformance")
                .cloned()
                .unwrap_or(Value::Null);

            json!({
                "isin": item.get("isin").cloned().unwrap_or(Value::Null),
                "name": item.get("name").cloned().unwrap_or(Value::Null),
                "security_type": item.get("type").cloned().unwrap_or(Value::Null),
                "quantity": position.get("filled").cloned().unwrap_or(Value::Null),
                "pending_quantity": position.get("pending").cloned().unwrap_or(Value::Null),
                "blocked_quantity": position.get("blocked").cloned().unwrap_or(Value::Null),
                "fifo_price": position.get("fifoPrice").cloned().unwrap_or(Value::Null),
                "valuation": performance.get("valuation").cloned().unwrap_or(Value::Null),
                "valuation_currency": performance.get("currency").cloned().unwrap_or(Value::Null),
                "quote_mid_price": quote.get("midPrice").cloned().unwrap_or(Value::Null),
                "quote_currency": quote.get("currency").cloned().unwrap_or(Value::Null),
                "quote_timestamp_utc": timestamp_value_or_null(quote.get("timestampUtc")),
                "quote_is_outdated": quote.get("isOutdated").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();

    projected.sort_by(|a, b| {
        let a_isin = a.get("isin").and_then(Value::as_str).unwrap_or_default();
        let b_isin = b.get("isin").and_then(Value::as_str).unwrap_or_default();
        a_isin.cmp(b_isin)
    });

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "count": projected.len(),
        "items": projected,
    }))
}

pub fn project_broker_transactions_response(
    input: &BrokerInput,
    response: &Value,
) -> Result<Value> {
    let summaries = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("moreTransactions"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.moreTransactions")
        })?;
    let items = summaries
        .get("transactions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!(
                "Broker response invalid: missing account.brokerPortfolio.moreTransactions.transactions"
            )
        })?;

    let projected_items = items
        .iter()
        .map(map_broker_transaction_summary)
        .collect::<Vec<_>>();

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "cursor": summaries.get("cursor").cloned().unwrap_or(Value::Null),
        "total": summaries.get("total").cloned().unwrap_or(Value::Null),
        "count": projected_items.len(),
        "items": projected_items,
    }))
}

pub fn project_broker_transaction_details_response(
    input: &BrokerInput,
    response: &Value,
) -> Result<Value> {
    let transaction = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("transactionDetails"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.transactionDetails")
        })?;

    let typename = transaction
        .get("__typename")
        .and_then(Value::as_str)
        .unwrap_or_default();

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "id": transaction.get("id").cloned().unwrap_or(Value::Null),
        "detail_type": map_broker_transaction_detail_type(typename),
        "detail_typename": transaction.get("__typename").cloned().unwrap_or(Value::Null),
        "type": transaction.get("type").cloned().unwrap_or(Value::Null),
        "currency": transaction.get("currency").cloned().unwrap_or(Value::Null),
        "transaction_reference": transaction
            .get("transactionReference")
            .cloned()
            .unwrap_or(Value::Null),
        "last_event_datetime": timestamp_value_or_null(transaction.get("lastEventDateTime")),
        "is_pending": transaction.get("isPending").cloned().unwrap_or(Value::Null),
        "is_cancellation": transaction.get("isCancellation").cloned().unwrap_or(Value::Null),
        "security": map_transaction_security_ref(transaction.get("security")),
        "documents": map_transaction_documents(transaction.get("documents")),
        "linked_transaction_ids": map_linked_transaction_ids(transaction.get("linkedTransactions")),
        "history": map_transaction_history_for_details(transaction),
        "security_trade": map_security_trade_details(transaction, typename),
        "cash": map_cash_transaction_details(transaction, typename),
        "non_trade_security": map_non_trade_security_details(transaction, typename),
        "eltif": map_eltif_transaction_details(transaction, typename),
    }))
}

pub fn project_broker_watchlist_response(input: &BrokerInput, response: &Value) -> Result<Value> {
    let items = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("watchlist"))
        .and_then(|v| v.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.watchlist.items")
        })?;

    let mut projected = items
        .iter()
        .map(|item| {
            let quote = item.get("quoteTick").cloned().unwrap_or(Value::Null);
            json!({
                "isin": item.get("isin").cloned().unwrap_or(Value::Null),
                "name": item.get("name").cloned().unwrap_or(Value::Null),
                "security_type": item.get("type").cloned().unwrap_or(Value::Null),
                "quote_mid_price": quote.get("midPrice").cloned().unwrap_or(Value::Null),
                "quote_currency": quote.get("currency").cloned().unwrap_or(Value::Null),
                "quote_timestamp_utc": timestamp_value_or_null(quote.get("timestampUtc")),
                "quote_is_outdated": quote.get("isOutdated").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();

    projected.sort_by(|a, b| {
        let a_isin = a.get("isin").and_then(Value::as_str).unwrap_or_default();
        let b_isin = b.get("isin").and_then(Value::as_str).unwrap_or_default();
        a_isin.cmp(b_isin)
    });

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "count": projected.len(),
        "items": projected,
    }))
}

pub fn project_broker_watchlist_add_response(
    requested_isin: &str,
    response: &Value,
) -> Result<Value> {
    project_broker_watchlist_mutation_response("add", requested_isin, "addToWatchlist", response)
}

pub fn project_broker_watchlist_remove_response(
    requested_isin: &str,
    response: &Value,
) -> Result<Value> {
    project_broker_watchlist_mutation_response(
        "remove",
        requested_isin,
        "removeFromWatchlist",
        response,
    )
}

pub fn project_broker_remove_savings_plan_response(
    requested_isin: &str,
    response: &Value,
) -> Result<Value> {
    let mutation = response
        .get("removeSavingsPlan")
        .ok_or_else(|| anyhow!("Broker response invalid: missing removeSavingsPlan"))?;
    let mutation_id = mutation
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Broker response invalid: missing removeSavingsPlan.id"))?;

    // The backend-selected id is only used as an acknowledgement contract check.
    let _ = mutation_id;

    Ok(json!({
        "action": "remove",
        "isin": requested_isin,
    }))
}

pub fn project_broker_search_response(
    input: &BrokerInput,
    search_term: &str,
    response: &Value,
) -> Result<Value> {
    let items = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("simpleSecuritySearch"))
        .and_then(|v| v.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Broker response invalid: missing account.brokerPortfolio.simpleSecuritySearch.items"))?;

    let mut projected = items
        .iter()
        .map(|item| {
            let quote = item.get("quoteTick").cloned().unwrap_or(Value::Null);
            json!({
                "isin": item.get("isin").cloned().unwrap_or(Value::Null),
                "name": item.get("name").cloned().unwrap_or(Value::Null),
                "security_type": item.get("type").cloned().unwrap_or(Value::Null),
                "quote_mid_price": quote.get("midPrice").cloned().unwrap_or(Value::Null),
                "quote_currency": quote.get("currency").cloned().unwrap_or(Value::Null),
                "quote_timestamp_utc": timestamp_value_or_null(quote.get("timestampUtc")),
                "quote_is_outdated": quote.get("isOutdated").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();

    projected.sort_by(|a, b| {
        let a_isin = a.get("isin").and_then(Value::as_str).unwrap_or_default();
        let b_isin = b.get("isin").and_then(Value::as_str).unwrap_or_default();
        a_isin.cmp(b_isin)
    });

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "query": search_term,
        "count": projected.len(),
        "items": projected,
    }))
}

pub fn project_broker_derivatives_search_response(
    input: &BrokerInput,
    query: &NormalizedBrokerDerivativesSearchQueryInput,
    response: &Value,
) -> Result<Value> {
    let derivatives_search = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("derivativesSearch"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.derivativesSearch")
        })?;
    let pagination = derivatives_search
        .get("pagination")
        .ok_or_else(|| anyhow!("Broker response invalid: missing derivatives search pagination"))?;
    let items = derivatives_search
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Broker response invalid: missing derivatives search results"))?;

    let projected = items
        .iter()
        .map(map_derivative_search_result)
        .collect::<Vec<_>>();

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "derivative_type": query.derivative_type.as_label(),
        "underlying_isin": query.underlying_isin,
        "offset": pagination.get("offset").cloned().unwrap_or_else(|| json!(query.offset)),
        "limit": pagination.get("limit").cloned().unwrap_or_else(|| json!(query.limit)),
        "total_available": pagination.get("totalAvailable").cloned().unwrap_or(Value::Null),
        "count": projected.len(),
        "items": projected,
    }))
}

pub fn project_broker_quote_response(
    input: &BrokerInput,
    requested_isin: &str,
    response: &Value,
) -> Result<Value> {
    let security = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("security"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.security")
        })?;
    let actual_isin = security
        .get("isin")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.security.isin")
        })?;
    if actual_isin != requested_isin {
        return Err(anyhow!(
            "Broker response invalid: requested isin '{requested_isin}' does not match returned security isin '{actual_isin}'"
        ));
    }

    let quote = security.get("quoteTick").cloned().unwrap_or(Value::Null);
    let mut performances = quote
        .get("performancesByTimeframe")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    performances.sort_by(|a, b| {
        let a_timeframe = a
            .get("timeframe")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let b_timeframe = b
            .get("timeframe")
            .and_then(Value::as_str)
            .unwrap_or_default();
        a_timeframe.cmp(b_timeframe)
    });
    let mapped_performances = performances
        .iter()
        .map(|entry| {
            json!({
                "timeframe": entry.get("timeframe").cloned().unwrap_or(Value::Null),
                "performance": entry.get("performance").cloned().unwrap_or(Value::Null),
                "simple_absolute_return": entry
                    .get("simpleAbsoluteReturn")
                    .cloned()
                    .unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "security_id": security.get("id").cloned().unwrap_or(Value::Null),
        "isin": actual_isin,
        "name": security.get("name").cloned().unwrap_or(Value::Null),
        "security_type": security.get("type").cloned().unwrap_or(Value::Null),
        "quote_tick_id": quote.get("id").cloned().unwrap_or(Value::Null),
        "quote_mid_price": quote.get("midPrice").cloned().unwrap_or(Value::Null),
        "quote_bid_price": quote.get("bidPrice").cloned().unwrap_or(Value::Null),
        "quote_ask_price": quote.get("askPrice").cloned().unwrap_or(Value::Null),
        "quote_currency": quote.get("currency").cloned().unwrap_or(Value::Null),
        "quote_timestamp_utc": timestamp_value_or_null(quote.get("timestampUtc")),
        "quote_is_outdated": quote.get("isOutdated").cloned().unwrap_or(Value::Null),
        "quote_performance_date": quote
            .get("performanceDate")
            .and_then(|v| v.get("date"))
            .cloned()
            .unwrap_or(Value::Null),
        "quote_performances": mapped_performances,
    }))
}

pub fn project_broker_security_news_response(
    isin: &str,
    locale: &str,
    response: &Value,
) -> Result<Value> {
    let news = response
        .get("securityNews")
        .ok_or_else(|| anyhow!("Broker response invalid: missing securityNews"))?;

    let mut sources = news
        .get("sources")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    sources.sort_by(|a, b| {
        let a_id = a.get("id").and_then(Value::as_str).unwrap_or_default();
        let b_id = b.get("id").and_then(Value::as_str).unwrap_or_default();
        a_id.cmp(b_id)
    });

    let mapped_sources = sources
        .iter()
        .map(|source| {
            json!({
                "id": source.get("id").cloned().unwrap_or(Value::Null),
                "headline": source.get("headline").cloned().unwrap_or(Value::Null),
                "source_name": source.get("sourceName").cloned().unwrap_or(Value::Null),
                "publication_time_utc": timestamp_value_or_null(source.get("publicationTime")),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "isin": isin,
        "locale": locale,
        "summary": {
            "short": news.get("shortNewsSummary").cloned().unwrap_or(Value::Null),
            "long": news.get("longNewsSummary").cloned().unwrap_or(Value::Null),
            "last_updated": timestamp_value_or_null(news.get("lastUpdated")),
        },
        "sources": mapped_sources,
    }))
}

pub fn project_broker_price_alerts_response(
    input: &BrokerInput,
    active_only: bool,
    response: &Value,
) -> Result<Value> {
    let groups = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("priceAlerts"))
        .and_then(|v| v.get("itemsPerInstrument"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Broker response invalid: missing account.brokerPortfolio.priceAlerts.itemsPerInstrument"))?;

    let mut items = Vec::new();
    for group in groups {
        let can_add_new = group.get("canAddNew").cloned().unwrap_or(Value::Null);
        if let Some(alerts) = group.get("items").and_then(Value::as_array) {
            for alert in alerts {
                let security = alert.get("security").cloned().unwrap_or(Value::Null);
                let triggered = alert
                    .get("triggeredTimestamp")
                    .and_then(|v| v.get("time"))
                    .cloned()
                    .unwrap_or(Value::Null);
                items.push(json!({
                    "alert_id": alert.get("id").cloned().unwrap_or(Value::Null),
                    "direction": alert.get("direction").cloned().unwrap_or(Value::Null),
                    "is_active": alert.get("isActive").cloned().unwrap_or(Value::Null),
                    "price": alert.get("price").cloned().unwrap_or(Value::Null),
                    "triggered_timestamp_utc": triggered,
                    "isin": security.get("isin").cloned().unwrap_or(Value::Null),
                    "name": security.get("name").cloned().unwrap_or(Value::Null),
                    "security_type": security.get("type").cloned().unwrap_or(Value::Null),
                    "can_add_new_for_instrument": can_add_new,
                }));
            }
        }
    }

    items.sort_by(|a, b| {
        let a_isin = a.get("isin").and_then(Value::as_str).unwrap_or_default();
        let b_isin = b.get("isin").and_then(Value::as_str).unwrap_or_default();
        let ord = a_isin.cmp(b_isin);
        if ord == std::cmp::Ordering::Equal {
            let a_id = a
                .get("alert_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let b_id = b
                .get("alert_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            a_id.cmp(b_id)
        } else {
            ord
        }
    });

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "active_only": active_only,
        "count": items.len(),
        "items": items,
    }))
}

fn map_derivative_search_result(item: &Value) -> Value {
    let expiry = map_derivative_expiry(item.get("expiryDate"));
    json!({
        "isin": item.get("isin").cloned().unwrap_or(Value::Null),
        "underlying_isin": item.get("underlyingIsin").cloned().unwrap_or(Value::Null),
        "issuer": item.get("issuer").cloned().unwrap_or(Value::Null),
        "strategy": item.get("strategy").cloned().unwrap_or(Value::Null),
        "product_subcategory": item.get("productSubcategory").cloned().unwrap_or(Value::Null),
        "leverage": item.get("leverage").cloned().unwrap_or(Value::Null),
        "factor": item.get("factor").cloned().unwrap_or(Value::Null),
        "omega": item.get("omega").cloned().unwrap_or(Value::Null),
        "delta": item.get("delta").cloned().unwrap_or(Value::Null),
        "implied_volatility": item.get("impliedVolatility").cloned().unwrap_or(Value::Null),
        "distance_to_knockout": item.get("distanceToKnockout").cloned().unwrap_or(Value::Null),
        "distance_to_strike": item.get("distanceToStrike").cloned().unwrap_or(Value::Null),
        "strike": map_derivative_price(item.get("strike")),
        "knockout_barrier": map_derivative_price(item.get("knockoutBarrier")),
        "premium_absolute": map_money_value(item.get("premiumAbsolute")),
        "premium_percentage": item.get("premiumPercentage").cloned().unwrap_or(Value::Null),
        "expiry_date": expiry.get("date").cloned().unwrap_or(Value::Null),
        "expiry_is_open_end": expiry.get("is_open_end").cloned().unwrap_or(Value::Null),
    })
}

fn map_derivative_price(raw: Option<&Value>) -> Value {
    let Some(raw) = raw else {
        return Value::Null;
    };

    let kind = match raw.get("__typename").and_then(Value::as_str) {
        Some("Money") => "money",
        Some("Point") => "point",
        _ if raw.get("currencyIsoCode").is_some() => "money",
        _ => "point",
    };

    json!({
        "kind": kind,
        "currency_iso_code": raw.get("currencyIsoCode").cloned().unwrap_or(Value::Null),
        "value": raw.get("value").cloned().unwrap_or(Value::Null),
    })
}

fn map_money_value(raw: Option<&Value>) -> Value {
    match raw {
        Some(raw) => json!({
            "currency_iso_code": raw.get("currencyIsoCode").cloned().unwrap_or(Value::Null),
            "value": raw.get("value").cloned().unwrap_or(Value::Null),
        }),
        None => Value::Null,
    }
}

fn map_derivative_expiry(raw: Option<&Value>) -> Value {
    let Some(raw) = raw else {
        return json!({
            "date": Value::Null,
            "is_open_end": Value::Null,
        });
    };

    let date = match raw.get("date") {
        Some(Value::Object(date)) => json!({
            "date": date.get("date").cloned().unwrap_or(Value::Null),
            "epoch_day": date.get("epochDay").cloned().unwrap_or(Value::Null),
        }),
        Some(Value::String(_)) | Some(Value::Number(_)) => json!({
            "date": raw.get("date").cloned().unwrap_or(Value::Null),
            "epoch_day": raw.get("epochDay").cloned().unwrap_or(Value::Null),
        }),
        _ if raw.get("epochDay").is_some() => json!({
            "date": Value::Null,
            "epoch_day": raw.get("epochDay").cloned().unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    };

    json!({
        "date": date,
        "is_open_end": raw.get("isOpenEnd").cloned().unwrap_or(Value::Null),
    })
}

pub fn project_broker_crypto_price_alerts_response(
    input: &BrokerInput,
    response: &Value,
) -> Result<Value> {
    let portfolio = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .ok_or_else(|| anyhow!("Broker response invalid: missing account.brokerPortfolio"))?;
    let groups = match portfolio
        .get("crypto")
        .and_then(|v| v.get("priceAlerts"))
        .and_then(|v| v.get("itemsPerInstrument"))
    {
        None | Some(Value::Null) => return Ok(empty_broker_crypto_price_alerts_projection(input)),
        Some(groups) => groups
            .as_array()
            .ok_or_else(|| {
                anyhow!(
                    "Broker response invalid: missing account.brokerPortfolio.crypto.priceAlerts.itemsPerInstrument"
                )
            })?,
    };

    let mut items = Vec::new();
    for group in groups {
        let can_add_new = group.get("canAddNew").cloned().unwrap_or(Value::Null);
        if let Some(alerts) = group.get("items").and_then(Value::as_array) {
            for alert in alerts {
                let coin = alert.get("coin").cloned().unwrap_or(Value::Null);
                let triggered = alert
                    .get("triggeredTimestamp")
                    .and_then(|v| v.get("time"))
                    .cloned()
                    .unwrap_or(Value::Null);
                items.push(json!({
                    "alert_id": alert.get("id").cloned().unwrap_or(Value::Null),
                    "direction": alert.get("direction").cloned().unwrap_or(Value::Null),
                    "is_active": alert.get("isActive").cloned().unwrap_or(Value::Null),
                    "price": alert.get("price").cloned().unwrap_or(Value::Null),
                    "triggered_timestamp_utc": triggered,
                    "ticker": coin.get("ticker").cloned().unwrap_or(Value::Null),
                    "name": coin.get("name").cloned().unwrap_or(Value::Null),
                    "can_add_new_for_instrument": can_add_new,
                }));
            }
        }
    }

    items.sort_by(|a, b| {
        let a_ticker = a.get("ticker").and_then(Value::as_str).unwrap_or_default();
        let b_ticker = b.get("ticker").and_then(Value::as_str).unwrap_or_default();
        let ord = a_ticker.cmp(b_ticker);
        if ord == std::cmp::Ordering::Equal {
            let a_id = a
                .get("alert_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let b_id = b
                .get("alert_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            a_id.cmp(b_id)
        } else {
            ord
        }
    });

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "count": items.len(),
        "items": items,
    }))
}

fn empty_broker_crypto_price_alerts_projection(input: &BrokerInput) -> Value {
    json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "count": 0,
        "items": [],
    })
}

pub fn project_broker_limits_response(input: &BrokerInput, response: &Value) -> Result<Value> {
    let portfolio = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .ok_or_else(|| anyhow!("Broker response invalid: missing account.brokerPortfolio"))?;

    let deposit_limits = portfolio
        .get("depositLimits")
        .cloned()
        .unwrap_or(Value::Null);
    let withdrawal_limits = portfolio
        .get("withdrawalLimits")
        .cloned()
        .unwrap_or(Value::Null);
    let payments = portfolio.get("payments").cloned().unwrap_or(Value::Null);

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "deposit_limits": {
            "min": deposit_limits.get("min").cloned().unwrap_or(Value::Null),
            "max": deposit_limits.get("max").cloned().unwrap_or(Value::Null),
        },
        "withdrawal_limits": {
            "min": withdrawal_limits.get("min").cloned().unwrap_or(Value::Null),
            "max": withdrawal_limits.get("max").cloned().unwrap_or(Value::Null),
            "max_excluding_credit": withdrawal_limits
                .get("maxExcludingCredit")
                .cloned()
                .unwrap_or(Value::Null),
        },
        "buying_power": payments.get("buyingPower").cloned().unwrap_or(Value::Null),
        "derivatives_buying_power": payments
            .get("derivativesBuyingPower")
            .cloned()
            .unwrap_or(Value::Null),
        "withdrawal_power": payments.get("withdrawalPower").cloned().unwrap_or(Value::Null),
    }))
}

pub fn project_broker_savings_plans_response(
    input: &BrokerInput,
    response: &Value,
) -> Result<Value> {
    let portfolio = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .ok_or_else(|| anyhow!("Broker response invalid: missing account.brokerPortfolio"))?;

    let total_savings_plan_amount = portfolio
        .get("totalSavingsPlanAmount")
        .cloned()
        .unwrap_or(Value::Null);

    let security_items = portfolio
        .get("inventory")
        .and_then(|v| v.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.inventory.items")
        })?;

    let mut non_crypto_items = security_items
        .iter()
        .filter_map(|item| {
            let savings_plan = item
                .get("inventory")
                .and_then(|v| v.get("savingsPlan"))
                .filter(|v| !v.is_null())?;
            let next_execution = savings_plan
                .get("nextExecutionDate")
                .cloned()
                .unwrap_or(Value::Null);

            Some(json!({
                "kind": "security",
                "isin": item.get("isin").cloned().unwrap_or(Value::Null),
                "name": item.get("name").cloned().unwrap_or(Value::Null),
                "security_type": item.get("type").cloned().unwrap_or(Value::Null),
                "amount": savings_plan.get("amount").cloned().unwrap_or(Value::Null),
                "frequency": savings_plan.get("frequency").cloned().unwrap_or(Value::Null),
                "day_of_month": savings_plan.get("dayOfTheMonth").cloned().unwrap_or(Value::Null),
                "dynamization_rate": savings_plan.get("dynamizationRate").cloned().unwrap_or(Value::Null),
                "payment_method": savings_plan.get("paymentMethod").cloned().unwrap_or(Value::Null),
                "next_execution_date": next_execution.get("date").cloned().unwrap_or(Value::Null),
                "next_execution_epoch_day": next_execution.get("epochDay").cloned().unwrap_or(Value::Null),
            }))
        })
        .collect::<Vec<_>>();

    let crypto_coins = portfolio
        .get("crypto")
        .and_then(|v| v.get("coins"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut crypto_items = crypto_coins
        .into_iter()
        .filter_map(|coin| {
            let amount = coin
                .get("savingsPlanAmount")
                .cloned()
                .unwrap_or(Value::Null);
            if !is_non_zero_decimal(&amount) {
                return None;
            }
            Some(json!({
                "kind": "crypto",
                "ticker": coin.get("ticker").cloned().unwrap_or(Value::Null),
                "name": coin.get("name").cloned().unwrap_or(Value::Null),
                "amount": amount,
            }))
        })
        .collect::<Vec<_>>();

    non_crypto_items.sort_by(|a, b| {
        let a_isin = a.get("isin").and_then(Value::as_str).unwrap_or_default();
        let b_isin = b.get("isin").and_then(Value::as_str).unwrap_or_default();
        a_isin.cmp(b_isin)
    });
    crypto_items.sort_by(|a, b| {
        let a_ticker = a.get("ticker").and_then(Value::as_str).unwrap_or_default();
        let b_ticker = b.get("ticker").and_then(Value::as_str).unwrap_or_default();
        a_ticker.cmp(b_ticker)
    });

    let non_crypto_count = non_crypto_items.len();
    let crypto_count = crypto_items.len();
    let mut items = non_crypto_items;
    items.extend(crypto_items);

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "total_savings_plan_amount": total_savings_plan_amount,
        "count": items.len(),
        "non_crypto_count": non_crypto_count,
        "crypto_count": crypto_count,
        "items": items,
    }))
}

fn is_non_zero_decimal(value: &Value) -> bool {
    match value {
        Value::Number(number) => number
            .as_f64()
            .map(|parsed| parsed.is_finite() && parsed > 0.0)
            .unwrap_or(false),
        Value::String(raw) => raw
            .trim()
            .parse::<f64>()
            .map(|parsed| parsed.is_finite() && parsed > 0.0)
            .unwrap_or(false),
        _ => false,
    }
}

fn map_broker_transaction_detail_type(typename: &str) -> Value {
    let detail_type = match typename {
        "BrokerSecurityTransaction" => "security_trade",
        "BrokerCashTransaction" => "cash",
        "BrokerNonTradeSecurityTransaction" => "non_trade_security",
        "BrokerEltifTransaction" => "eltif",
        _ => "unknown",
    };

    Value::String(detail_type.to_string())
}

fn map_transaction_security_ref(raw: Option<&Value>) -> Value {
    match raw {
        Some(security) if !security.is_null() => json!({
            "isin": security.get("isin").cloned().unwrap_or(Value::Null),
            "name": security.get("name").cloned().unwrap_or(Value::Null),
            "security_type": security.get("type").cloned().unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    }
}

fn map_transaction_documents(raw: Option<&Value>) -> Value {
    raw.and_then(Value::as_array)
        .map(|items| {
            Value::Array(
                items
                    .iter()
                    .map(|item| {
                        json!({
                            "id": item.get("id").cloned().unwrap_or(Value::Null),
                            "url": item.get("url").cloned().unwrap_or(Value::Null),
                            "label": item.get("label").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect(),
            )
        })
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

fn map_linked_transaction_ids(raw: Option<&Value>) -> Value {
    raw.and_then(Value::as_array)
        .map(|items| {
            Value::Array(
                items
                    .iter()
                    .map(|item| item.get("id").cloned().unwrap_or(Value::Null))
                    .collect(),
            )
        })
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

fn map_transaction_history_for_details(transaction: &Value) -> Value {
    let history = transaction
        .get("securityTransactionHistory")
        .or_else(|| transaction.get("cashTransactionHistory"))
        .or_else(|| transaction.get("nonTradeSecurityTransactionHistory"))
        .or_else(|| transaction.get("eltifTransactionHistory"))
        .or_else(|| transaction.get("transactionHistory"));

    history
        .and_then(Value::as_array)
        .map(|items| {
            Value::Array(
                items.iter()
                    .map(|item| {
                        json!({
                            "state": item.get("state").cloned().unwrap_or(Value::Null),
                            "timestamp": timestamp_value_or_null(item.get("time").or_else(|| item.get("timestamp"))),
                            "number_of_shares": map_number_of_shares(item.get("numberOfShares")),
                            "execution_price": item.get("executionPrice").cloned().unwrap_or(Value::Null),
                            "amount": item.get("amount").cloned().unwrap_or(Value::Null),
                            "eltif_quantity": item.get("eltifQuantity").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect(),
            )
        })
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

fn map_security_trade_details(transaction: &Value, typename: &str) -> Value {
    if typename != "BrokerSecurityTransaction" {
        return Value::Null;
    }

    json!({
        "status": transaction.get("status").cloned().unwrap_or(Value::Null),
        "side": transaction.get("side").cloned().unwrap_or(Value::Null),
        "order_kind": transaction.get("orderKind").cloned().unwrap_or(Value::Null),
        "number_of_shares": map_number_of_shares(transaction.get("numberOfShares")),
        "average_price": transaction.get("averagePrice").cloned().unwrap_or(Value::Null),
        "total_amount": transaction.get("totalAmount").cloned().unwrap_or(Value::Null),
        "finalisation_reason": transaction
            .get("finalisationReason")
            .cloned()
            .unwrap_or(Value::Null),
        "limit_price": transaction.get("limitPrice").cloned().unwrap_or(Value::Null),
        "stop_price": transaction.get("stopPrice").cloned().unwrap_or(Value::Null),
        "valid_until": timestamp_value_or_null(transaction.get("validUntil")),
        "is_cancellation_requested": transaction
            .get("isCancellationRequested")
            .cloned()
            .unwrap_or(Value::Null),
        "trading_venue": transaction.get("tradingVenue").cloned().unwrap_or(Value::Null),
        "fee": transaction.get("fee").cloned().unwrap_or(Value::Null),
        "transactional_fee": transaction
            .get("transactionalFee")
            .cloned()
            .unwrap_or(Value::Null),
        "taxes": transaction.get("taxes").cloned().unwrap_or(Value::Null),
        "trade_transaction_amounts": map_trade_transaction_amounts(
            transaction.get("tradeTransactionAmounts"),
        ),
        "aggregated_transaction_taxes": map_aggregated_transaction_taxes(
            transaction.get("aggregatedTransactionTaxes"),
        ),
        "trailing_stop_info": map_trailing_stop_info(transaction.get("trailingStopInfo")),
    })
}

fn map_cash_transaction_details(transaction: &Value, typename: &str) -> Value {
    if typename != "BrokerCashTransaction" {
        return Value::Null;
    }

    json!({
        "cash_transaction_type": transaction
            .get("cashTransactionType")
            .cloned()
            .unwrap_or(Value::Null),
        "amount": transaction.get("amount").cloned().unwrap_or(Value::Null),
        "description": transaction.get("description").cloned().unwrap_or(Value::Null),
        "tax_details": map_cash_tax_details(transaction.get("taxDetails")),
        "sddi_details": map_sddi_details(transaction.get("sddiDetails")),
    })
}

fn map_non_trade_security_details(transaction: &Value, typename: &str) -> Value {
    if typename != "BrokerNonTradeSecurityTransaction" {
        return Value::Null;
    }

    json!({
        "isin": transaction.get("isin").cloned().unwrap_or(Value::Null),
        "non_trade_security_transaction_type": transaction
            .get("nonTradeSecurityTransactionType")
            .cloned()
            .unwrap_or(Value::Null),
        "quantity": transaction.get("quantity").cloned().unwrap_or(Value::Null),
        "average_price": transaction
            .get("nonTradeAveragePrice")
            .or_else(|| transaction.get("averagePrice"))
            .cloned()
            .unwrap_or(Value::Null),
        "total_amount": transaction
            .get("nonTradeSecurityAmount")
            .or_else(|| transaction.get("totalAmount"))
            .cloned()
            .unwrap_or(Value::Null),
        "description": transaction.get("description").cloned().unwrap_or(Value::Null),
    })
}

fn map_eltif_transaction_details(transaction: &Value, typename: &str) -> Value {
    if typename != "BrokerEltifTransaction" {
        return Value::Null;
    }

    json!({
        "status": transaction.get("status").cloned().unwrap_or(Value::Null),
        "side": transaction.get("side").cloned().unwrap_or(Value::Null),
        "order_kind": transaction.get("orderKind").cloned().unwrap_or(Value::Null),
        "amount": transaction.get("amount").cloned().unwrap_or(Value::Null),
        "finalisation_reason": transaction
            .get("finalisationReason")
            .cloned()
            .unwrap_or(Value::Null),
        "eltif_quantity": transaction.get("eltifQuantity").cloned().unwrap_or(Value::Null),
        "execution_price": transaction.get("executionPrice").cloned().unwrap_or(Value::Null),
        "execution_date": timestamp_value_or_null(transaction.get("executionDate")),
        "earliest_sell_date": timestamp_value_or_null(transaction.get("earliestSellDate")),
        "market_valuation": transaction.get("marketValuation").cloned().unwrap_or(Value::Null),
        "trading_venue": transaction.get("tradingVenue").cloned().unwrap_or(Value::Null),
        "is_multiple_orders_cancellation": transaction
            .get("isMultipleOrdersCancellation")
            .cloned()
            .unwrap_or(Value::Null),
        "is_initial_investment": transaction
            .get("isInitialInvestment")
            .cloned()
            .unwrap_or(Value::Null),
        "cancelable_details": map_cancelable_details(transaction.get("cancelableDetails")),
    })
}

fn map_number_of_shares(raw: Option<&Value>) -> Value {
    match raw {
        Some(value) if !value.is_null() => json!({
            "filled": value.get("filled").cloned().unwrap_or(Value::Null),
            "total": value.get("total").cloned().unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    }
}

fn map_trade_transaction_amounts(raw: Option<&Value>) -> Value {
    match raw {
        Some(value) if !value.is_null() => json!({
            "market_valuation": value.get("marketValuation").cloned().unwrap_or(Value::Null),
            "tax_amount": value.get("taxAmount").cloned().unwrap_or(Value::Null),
            "transaction_fee": value.get("transactionFee").cloned().unwrap_or(Value::Null),
            "venue_fee": value.get("venueFee").cloned().unwrap_or(Value::Null),
            "crypto_spread_fee": value.get("cryptoSpreadFee").cloned().unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    }
}

fn map_aggregated_transaction_taxes(raw: Option<&Value>) -> Value {
    match raw {
        Some(value) if !value.is_null() => json!({
            "total_tax": value.get("totalTax").cloned().unwrap_or(Value::Null),
            "capital_gains_tax": value.get("capitalGainsTax").cloned().unwrap_or(Value::Null),
            "church_tax": value.get("churchTax").cloned().unwrap_or(Value::Null),
            "solidarity_tax": value.get("solidarityTax").cloned().unwrap_or(Value::Null),
            "source_tax": value.get("sourceTax").cloned().unwrap_or(Value::Null),
            "financial_transaction_tax": value
                .get("financialTransactionTax")
                .cloned()
                .unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    }
}

fn map_trailing_stop_info(raw: Option<&Value>) -> Value {
    match raw {
        Some(value) if !value.is_null() => json!({
            "trail_type": value.get("trailType").cloned().unwrap_or(Value::Null),
            "trail_offset": value.get("trailOffset").cloned().unwrap_or(Value::Null),
            "latest_stop_price_timestamp": timestamp_value_or_null(value.get("latestStopPriceTimestamp")),
            "latest_stop_price_epoch_second": value
                .get("latestStopPriceTimestamp")
                .and_then(|v| v.get("epochSecond"))
                .cloned()
                .unwrap_or(Value::Null),
            "latest_stop_price_epoch_millisecond": value
                .get("latestStopPriceTimestamp")
                .and_then(|v| v.get("epochMillisecond"))
                .cloned()
                .unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    }
}

fn map_cash_tax_details(raw: Option<&Value>) -> Value {
    match raw {
        Some(value) if !value.is_null() => json!({
            "gross_amount": value.get("grossAmount").cloned().unwrap_or(Value::Null),
            "tax_amount": value.get("taxAmount").cloned().unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    }
}

fn map_sddi_details(raw: Option<&Value>) -> Value {
    match raw {
        Some(value) if !value.is_null() => json!({
            "fee": value.get("fee").cloned().unwrap_or(Value::Null),
            "gross_amount": value.get("grossAmount").cloned().unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    }
}

fn map_cancelable_details(raw: Option<&Value>) -> Value {
    match raw {
        Some(value) if !value.is_null() => json!({
            "days_left": value.get("daysLeft").cloned().unwrap_or(Value::Null),
            "is_cancelable": value.get("isCancelable").cloned().unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    }
}

fn map_broker_transaction_summary(item: &Value) -> Value {
    let typename = item
        .get("__typename")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut mapped = json!({
        "id": item.get("id").cloned().unwrap_or(Value::Null),
        "summary_type": item.get("__typename").cloned().unwrap_or(Value::Null),
        "currency": item.get("currency").cloned().unwrap_or(Value::Null),
        "type": item.get("type").cloned().unwrap_or(Value::Null),
        "status": item.get("status").cloned().unwrap_or(Value::Null),
        "is_cancellation": item.get("isCancellation").cloned().unwrap_or(Value::Null),
        "last_event_datetime": timestamp_value_or_null(item.get("lastEventDateTime")),
        "description": item.get("description").cloned().unwrap_or(Value::Null),
        "custodian": item.get("custodian").cloned().unwrap_or(Value::Null),
        "documents": item
            .get("documents")
            .and_then(Value::as_array)
            .cloned()
            .map(Value::Array)
            .unwrap_or_else(|| Value::Array(Vec::new())),
        "unknown_summary_type": false,
    });

    if let Some(obj) = mapped.as_object_mut() {
        match typename {
            "BrokerSecurityTransactionSummary" => {
                obj.insert(
                    "isin".to_string(),
                    item.get("isin").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "security_transaction_type".to_string(),
                    item.get("securityTransactionType")
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                obj.insert(
                    "quantity".to_string(),
                    item.get("quantity").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "amount".to_string(),
                    item.get("amount").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "side".to_string(),
                    item.get("side").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "limit_price".to_string(),
                    item.get("limitPrice").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "stop_price".to_string(),
                    item.get("stopPrice").cloned().unwrap_or(Value::Null),
                );
            }
            "BrokerCashTransactionSummary" => {
                obj.insert(
                    "related_isin".to_string(),
                    item.get("relatedIsin").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "cash_transaction_type".to_string(),
                    item.get("cashTransactionType")
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                obj.insert(
                    "amount".to_string(),
                    item.get("amount").cloned().unwrap_or(Value::Null),
                );
            }
            "BrokerNonTradeSecurityTransactionSummary" => {
                obj.insert(
                    "isin".to_string(),
                    item.get("isin").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "non_trade_security_transaction_type".to_string(),
                    item.get("nonTradeSecurityTransactionType")
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                obj.insert(
                    "quantity".to_string(),
                    item.get("quantity").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "amount".to_string(),
                    item.get("amount").cloned().unwrap_or(Value::Null),
                );
            }
            "BrokerEltifTransactionSummary" => {
                obj.insert(
                    "isin".to_string(),
                    item.get("isin").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "security_transaction_type".to_string(),
                    item.get("securityTransactionType")
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                obj.insert(
                    "eltif_quantity".to_string(),
                    item.get("eltifQuantity").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "amount".to_string(),
                    item.get("amount").cloned().unwrap_or(Value::Null),
                );
                obj.insert(
                    "side".to_string(),
                    item.get("side").cloned().unwrap_or(Value::Null),
                );
            }
            _ => {
                obj.insert("unknown_summary_type".to_string(), Value::Bool(true));
            }
        }
    }

    mapped
}

fn project_broker_watchlist_mutation_response(
    action: &'static str,
    requested_isin: &str,
    mutation_field: &'static str,
    response: &Value,
) -> Result<Value> {
    let security = response
        .get(mutation_field)
        .and_then(|value| value.get("security"))
        .ok_or_else(|| anyhow!("Broker response invalid: missing {mutation_field}.security"))?;

    let _response_isin = security
        .get("isin")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing {mutation_field}.security.isin")
        })?;

    let is_on_watchlist = security.get("isOnWatchlist").cloned().ok_or_else(|| {
        anyhow!("Broker response invalid: missing {mutation_field}.security.isOnWatchlist")
    })?;

    Ok(json!({
        "action": action,
        "isin": requested_isin,
        "is_on_watchlist": is_on_watchlist,
    }))
}

fn project_broker_remove_price_alert_mutation_response(
    mutation_field: &'static str,
    kind: &'static str,
    requested_alert_id: &str,
    instrument_field: &'static str,
    instrument_value: &str,
    response: &Value,
) -> Result<Value> {
    response
        .get(mutation_field)
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Broker response invalid: missing {mutation_field}.id"))?;

    let mut result = serde_json::Map::new();
    result.insert("action".to_string(), Value::String("remove".to_string()));
    result.insert("kind".to_string(), Value::String(kind.to_string()));
    result.insert(
        "alert_id".to_string(),
        Value::String(requested_alert_id.to_string()),
    );
    result.insert(
        instrument_field.to_string(),
        Value::String(instrument_value.to_string()),
    );
    result.insert("removed".to_string(), Value::Bool(true));

    Ok(Value::Object(result))
}

pub fn project_broker_savings_plan_config_response(response: &Value) -> Result<Value> {
    response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("security"))
        .and_then(|v| v.get("savingsPlanConfiguration"))
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "Broker response invalid: missing account.brokerPortfolio.security.savingsPlanConfiguration"
            )
        })
}

pub fn project_broker_create_or_update_savings_plan_response(response: &Value) -> Result<Value> {
    let mutation = response
        .get("createOrUpdateSavingsPlan")
        .ok_or_else(|| anyhow!("Broker response invalid: missing createOrUpdateSavingsPlan"))?;
    Ok(json!({
        "mutation_id": mutation.get("id").cloned().unwrap_or(Value::Null),
    }))
}

pub fn project_broker_savings_plan_by_isin_response(response: &Value) -> Result<Value> {
    let security = response
        .get("account")
        .and_then(|v| v.get("brokerPortfolio"))
        .and_then(|v| v.get("security"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.security")
        })?;

    let mapped_savings_plan = security
        .get("inventory")
        .and_then(|v| v.get("savingsPlan"))
        .filter(|v| !v.is_null())
        .map(map_broker_savings_plan);

    Ok(json!({
        "security": {
            "isin": security.get("isin").cloned().unwrap_or(Value::Null),
            "name": security.get("name").cloned().unwrap_or(Value::Null),
            "security_type": security.get("type").cloned().unwrap_or(Value::Null),
        },
        "savings_plan": mapped_savings_plan.unwrap_or(Value::Null),
    }))
}

fn map_broker_savings_plan(savings_plan: &Value) -> Value {
    let next_execution = savings_plan
        .get("nextExecutionDate")
        .cloned()
        .unwrap_or(Value::Null);
    json!({
        "isin": savings_plan.get("isin").cloned().unwrap_or(Value::Null),
        "amount": savings_plan.get("amount").cloned().unwrap_or(Value::Null),
        "frequency": savings_plan.get("frequency").cloned().unwrap_or(Value::Null),
        "day_of_month": savings_plan.get("dayOfTheMonth").cloned().unwrap_or(Value::Null),
        "dynamization_rate": savings_plan.get("dynamizationRate").cloned().unwrap_or(Value::Null),
        "payment_method": savings_plan.get("paymentMethod").cloned().unwrap_or(Value::Null),
        "next_execution_date": next_execution.get("date").cloned().unwrap_or(Value::Null),
        "next_execution_epoch_day": next_execution.get("epochDay").cloned().unwrap_or(Value::Null),
    })
}

fn map_security_ref(security: &Value) -> Value {
    json!({
        "id": security.get("id").cloned().unwrap_or(Value::Null),
        "isin": security.get("isin").cloned().unwrap_or(Value::Null),
        "name": security.get("name").cloned().unwrap_or(Value::Null),
        "security_type": security.get("type").cloned().unwrap_or(Value::Null),
    })
}

fn map_health_check(health_check: &Value) -> Value {
    json!({
        "id": health_check.get("id").cloned().unwrap_or(Value::Null),
        "type": health_check.get("type").cloned().unwrap_or(Value::Null),
        "health_score": health_check.get("healthScore").cloned().unwrap_or(Value::Null),
        "state": health_check.get("state").cloned().unwrap_or(Value::Null),
        "color_hex": health_check
            .get("color")
            .and_then(|v| v.get("sixDigitNotationValue"))
            .cloned()
            .unwrap_or(Value::Null),
        "number_of_items_in_portfolio": health_check
            .get("numberOfItemsInPortfolio")
            .cloned()
            .unwrap_or(Value::Null),
        "max_items": health_check.get("maxItems").cloned().unwrap_or(Value::Null),
    })
}

fn map_portfolio_scenario(scenario: &Value) -> Value {
    let securities = scenario
        .get("securities")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(map_security_ref).collect::<Vec<_>>())
        .unwrap_or_default();

    json!({
        "id": scenario.get("id").cloned().unwrap_or(Value::Null),
        "type": scenario.get("type").cloned().unwrap_or(Value::Null),
        "portfolio_performance": scenario
            .get("portfolioPerformance")
            .cloned()
            .unwrap_or(Value::Null),
        "benchmark_performance": scenario
            .get("benchmarkPerformance")
            .cloned()
            .unwrap_or(Value::Null),
        "securities": securities,
    })
}

fn map_portfolio_allocation(allocation: &Value) -> Value {
    let positions = allocation
        .get("positions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(map_allocation_position)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "id": allocation.get("id").cloned().unwrap_or(Value::Null),
        "type": allocation.get("type").cloned().unwrap_or(Value::Null),
        "positions": positions,
    })
}

fn map_allocation_position(position: &Value) -> Value {
    let contributors = position
        .get("contributors")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(map_allocation_contributor)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let subpositions = position
        .get("subpositions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(map_allocation_position)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "id": position.get("id").cloned().unwrap_or(Value::Null),
        "name": position.get("name").cloned().unwrap_or(Value::Null),
        "weight": position.get("weight").cloned().unwrap_or(Value::Null),
        "valuation": position.get("valuation").cloned().unwrap_or(Value::Null),
        "contributors": contributors,
        "subpositions": subpositions,
    })
}

fn map_allocation_contributor(contributor: &Value) -> Value {
    json!({
        "id": contributor.get("id").cloned().unwrap_or(Value::Null),
        "weight": contributor.get("weight").cloned().unwrap_or(Value::Null),
        "underlying_asset": map_underlying_asset(contributor.get("underlyingAsset")),
    })
}

fn map_underlying_asset(asset: Option<&Value>) -> Value {
    match asset {
        Some(value) if value.get("isin").is_some() => json!({
            "kind": "security",
            "isin": value.get("isin").cloned().unwrap_or(Value::Null),
            "name": value.get("name").cloned().unwrap_or(Value::Null),
            "filled_quantity": value
                .get("inventory")
                .and_then(|v| v.get("position"))
                .and_then(|v| v.get("filled"))
                .cloned()
                .unwrap_or(Value::Null),
        }),
        Some(value) if value.get("ticker").is_some() => json!({
            "kind": "crypto",
            "ticker": value.get("ticker").cloned().unwrap_or(Value::Null),
            "name": value.get("name").cloned().unwrap_or(Value::Null),
        }),
        Some(value) => value.clone(),
        None => Value::Null,
    }
}

fn map_equity_company_styles(styles: Option<&Value>) -> Value {
    match styles {
        Some(value) => {
            let market_caps = value
                .get("marketCaps")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(map_market_cap).collect::<Vec<_>>())
                .unwrap_or_default();

            json!({
                "id": value.get("id").cloned().unwrap_or(Value::Null),
                "market_caps": market_caps,
            })
        }
        None => Value::Null,
    }
}

fn map_market_cap(market_cap: &Value) -> Value {
    let items = market_cap
        .get("items")
        .and_then(Value::as_array)
        .map(|values| values.iter().map(map_market_cap_item).collect::<Vec<_>>())
        .unwrap_or_default();
    let contributors = market_cap
        .get("contributors")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(map_allocation_contributor)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "id": market_cap.get("id").cloned().unwrap_or(Value::Null),
        "type": market_cap.get("type").cloned().unwrap_or(Value::Null),
        "weight": market_cap.get("weight").cloned().unwrap_or(Value::Null),
        "items": items,
        "contributors": contributors,
    })
}

fn map_market_cap_item(item: &Value) -> Value {
    let contributors = item
        .get("contributors")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(map_allocation_contributor)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "id": item.get("id").cloned().unwrap_or(Value::Null),
        "type": item.get("type").cloned().unwrap_or(Value::Null),
        "weight": item.get("weight").cloned().unwrap_or(Value::Null),
        "contributors": contributors,
    })
}

fn map_fixed_income_ratings(ratings: Option<&Value>) -> Value {
    match ratings {
        Some(value) => json!({
            "id": value.get("id").cloned().unwrap_or(Value::Null),
            "investment_grade": value
                .get("investmentGrade")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(map_fixed_income_rating_grade).collect::<Vec<_>>())
                .unwrap_or_default(),
            "speculative_grade": value
                .get("speculativeGrade")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(map_fixed_income_rating_grade).collect::<Vec<_>>())
                .unwrap_or_default(),
            "unrated_grade": value
                .get("unratedGrade")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(map_fixed_income_rating_grade).collect::<Vec<_>>())
                .unwrap_or_default(),
            "show_speculative_investment_warning": value
                .get("showSpeculativeInvestmentWarning")
                .cloned()
                .unwrap_or(Value::Null),
        }),
        None => Value::Null,
    }
}

fn map_fixed_income_rating_grade(grade: &Value) -> Value {
    let contributors = grade
        .get("contributors")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(map_allocation_contributor)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "id": grade.get("id").cloned().unwrap_or(Value::Null),
        "name": grade.get("name").cloned().unwrap_or(Value::Null),
        "weight": grade.get("weight").cloned().unwrap_or(Value::Null),
        "contributors": contributors,
    })
}

fn map_payments(payments: Option<&Value>) -> Value {
    match payments {
        Some(value) if value.is_null() => Value::Null,
        Some(value) => json!({
            "id": value.get("id").cloned().unwrap_or(Value::Null),
            "total_distributions": value
                .get("totalDistributions")
                .cloned()
                .unwrap_or(Value::Null),
            "total_interest": value.get("totalInterest").cloned().unwrap_or(Value::Null),
        }),
        None => Value::Null,
    }
}

fn map_trial_period(trial_period: Option<&Value>) -> Value {
    match trial_period {
        Some(value) if value.is_null() => Value::Null,
        Some(value) => json!({
            "id": value.get("id").cloned().unwrap_or(Value::Null),
            "is_eligible": value.get("isEligible").cloned().unwrap_or(Value::Null),
            "is_running": value.get("isRunning").cloned().unwrap_or(Value::Null),
            "start_time_utc": timestamp_value_or_null(value.get("startDate")),
            "duration_hours": value.get("durationInHours").cloned().unwrap_or(Value::Null),
        }),
        None => Value::Null,
    }
}

pub fn project_broker_add_price_alert_response(
    input: &BrokerInput,
    isin: &str,
    requested_price: &str,
    response: &Value,
) -> Result<Value> {
    let price_alerts = response
        .get("addPriceAlert")
        .and_then(|v| v.get("security"))
        .and_then(|v| v.get("priceAlerts"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing addPriceAlert.security.priceAlerts")
        })?;

    let can_add_new = price_alerts
        .get("canAddNew")
        .cloned()
        .unwrap_or(Value::Null);
    let mut items = price_alerts
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|alert| {
            let security = alert.get("security").cloned().unwrap_or(Value::Null);
            json!({
                "alert_id": alert.get("id").cloned().unwrap_or(Value::Null),
                "direction": alert.get("direction").cloned().unwrap_or(Value::Null),
                "is_active": alert.get("isActive").cloned().unwrap_or(Value::Null),
                "price": alert.get("price").cloned().unwrap_or(Value::Null),
                "triggered_timestamp_utc": timestamp_value_or_null(alert.get("triggeredTimestamp")),
                "isin": security.get("isin").cloned().unwrap_or(Value::String(isin.to_string())),
                "name": security.get("name").cloned().unwrap_or(Value::Null),
                "security_type": security.get("type").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();

    items.sort_by(|a, b| {
        let a_id = a
            .get("alert_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let b_id = b
            .get("alert_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        a_id.cmp(b_id)
    });

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "kind": "security",
        "isin": isin,
        "requested_price": requested_price,
        "can_add_new_for_instrument": can_add_new,
        "count": items.len(),
        "items": items,
    }))
}

pub fn project_broker_add_crypto_price_alert_response(
    input: &BrokerInput,
    ticker: &str,
    requested_price: &str,
    response: &Value,
) -> Result<Value> {
    let coin = response
        .get("addCryptoPriceAlert")
        .and_then(|v| v.get("crypto"))
        .and_then(|v| v.get("coin"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing addCryptoPriceAlert.crypto.coin")
        })?;

    let price_alerts = coin.get("priceAlerts").ok_or_else(|| {
        anyhow!("Broker response invalid: missing addCryptoPriceAlert.crypto.coin.priceAlerts")
    })?;

    let can_add_new = price_alerts
        .get("canAddNew")
        .cloned()
        .unwrap_or(Value::Null);
    let mut items = price_alerts
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|alert| {
            let alert_coin = alert.get("coin").cloned().unwrap_or(Value::Null);
            json!({
                "alert_id": alert.get("id").cloned().unwrap_or(Value::Null),
                "direction": alert.get("direction").cloned().unwrap_or(Value::Null),
                "is_active": alert.get("isActive").cloned().unwrap_or(Value::Null),
                "price": alert.get("price").cloned().unwrap_or(Value::Null),
                "triggered_timestamp_utc": timestamp_value_or_null(alert.get("triggeredTimestamp")),
                "ticker": alert_coin.get("ticker").cloned().unwrap_or(Value::String(ticker.to_string())),
                "name": alert_coin.get("name").cloned().unwrap_or_else(|| coin.get("name").cloned().unwrap_or(Value::Null)),
            })
        })
        .collect::<Vec<_>>();

    items.sort_by(|a, b| {
        let a_id = a
            .get("alert_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let b_id = b
            .get("alert_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        a_id.cmp(b_id)
    });

    Ok(json!({
        "account_id": input.account_id_value(),
        "portfolio_id": input.portfolio_id_value(),
        "kind": "crypto",
        "ticker": ticker,
        "requested_price": requested_price,
        "can_add_new_for_instrument": can_add_new,
        "count": items.len(),
        "items": items,
    }))
}

pub fn project_broker_remove_price_alert_response(
    alert_id: &str,
    isin: &str,
    response: &Value,
) -> Result<Value> {
    project_broker_remove_price_alert_mutation_response(
        "removePriceAlert",
        "security",
        alert_id,
        "isin",
        isin,
        response,
    )
}

pub fn project_broker_remove_crypto_price_alert_response(
    alert_id: &str,
    ticker: &str,
    response: &Value,
) -> Result<Value> {
    project_broker_remove_price_alert_mutation_response(
        "removeCryptoPriceAlert",
        "crypto",
        alert_id,
        "ticker",
        ticker,
        response,
    )
}

#[cfg(test)]
#[path = "broker_projections_tests.rs"]
mod tests;
