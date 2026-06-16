use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::broker_context::{
    BrokerContext, context_file_path, load_context as load_broker_context,
    save_context as save_broker_context,
};
use crate::broker_query_execution::{
    execute_broker_analytics as execute_broker_analytics_query,
    execute_broker_derivatives_search as execute_broker_derivatives_search_query,
    execute_broker_holdings as execute_broker_holdings_query,
    execute_broker_overview as execute_broker_overview_query,
    execute_broker_price_alerts as execute_broker_price_alerts_query,
    execute_broker_quote as execute_broker_quote_query,
    execute_broker_savings_plans as execute_broker_savings_plans_query,
    execute_broker_search as execute_broker_search_query,
    execute_broker_security_news as execute_broker_security_news_query,
    execute_broker_transaction_details as execute_broker_transaction_details_query,
    execute_broker_transactions as execute_broker_transactions_query,
    execute_broker_watchlist as execute_broker_watchlist_query,
};
use crate::broker_shared::{
    RESOLVE_BROKER_IDS_QUERY, load_active_session, resolve_broker_ids, validated_broker_input,
};
use crate::cli::{
    BrokerArgs, BrokerCommand, BrokerContextCommand, BrokerDerivativesCommand,
    BrokerPriceAlertsCommand, BrokerSavingsPlansCommand, BrokerTradeCommand,
    BrokerTransactionCommand, BrokerWatchlistCommand,
};
use crate::config::{AppConfig, EnvConfig, TargetEnv};
use crate::graphql::execute_graphql;
use crate::helpers::{
    BROKER_ADD_CRYPTO_PRICE_ALERT_MUTATION, BROKER_ADD_PRICE_ALERT_MUTATION,
    BROKER_ADD_TO_WATCHLIST_MUTATION, BROKER_CREATE_OR_UPDATE_SAVINGS_PLAN_MUTATION,
    BROKER_CRYPTO_PRICE_ALERTS_QUERY, BROKER_PRICE_ALERTS_QUERY,
    BROKER_REMOVE_CRYPTO_PRICE_ALERT_MUTATION, BROKER_REMOVE_FROM_WATCHLIST_MUTATION,
    BROKER_REMOVE_PRICE_ALERT_MUTATION, BROKER_REMOVE_SAVINGS_PLAN_MUTATION,
    BROKER_SAVINGS_PLAN_BY_ISIN_QUERY, BROKER_SAVINGS_PLAN_CONFIG_QUERY,
    broker_add_crypto_price_alert_variables, broker_add_price_alert_variables,
    broker_add_to_watchlist_variables, broker_create_or_update_savings_plan_variables,
    broker_crypto_price_alerts_variables, broker_remove_from_watchlist_variables,
    broker_remove_price_alert_variables, broker_remove_savings_plan_variables,
    broker_savings_plan_by_isin_variables, broker_savings_plan_config_variables,
    project_broker_add_crypto_price_alert_response, project_broker_add_price_alert_response,
    project_broker_create_or_update_savings_plan_response,
    project_broker_crypto_price_alerts_response, project_broker_remove_crypto_price_alert_response,
    project_broker_remove_price_alert_response, project_broker_remove_savings_plan_response,
    project_broker_savings_plan_by_isin_response, project_broker_savings_plan_config_response,
    project_broker_watchlist_add_response, project_broker_watchlist_remove_response,
};
use crate::session::{Session, SessionManager};
use crate::trade_execution::{
    execute_broker_trade_buy, execute_broker_trade_cancel, execute_broker_trade_sell,
    render_trade_buy_text, render_trade_cancel_text, render_trade_sell_text,
};
use crate::{execute_with_refresh_retry, resolve_active_env};

pub(crate) enum HumanBrokerOutput {
    Json(Value, bool),
    Text(Vec<String>),
}

pub(crate) fn bootstrap_broker_context_after_login(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    dpop_options: &crate::dpop::DpopRuntimeOptions,
) -> Result<BrokerContext> {
    let stored = session_manager.load_required_active()?;
    if stored.env != env {
        bail!("No active session for {env} after login");
    }
    let mut session = stored.session;
    let access_context = crate::graphql::GraphqlAccessContext::with_session_mode(stored.mode);

    // Always bootstrap at least account_id from the authenticated session.
    let mut context = BrokerContext {
        account_id: session.person_id.clone(),
        portfolio_id: None,
    };

    let person_id = session.person_id.clone();
    if let Ok(response) = execute_with_refresh_retry(
        session_manager,
        env,
        env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                RESOLVE_BROKER_IDS_QUERY,
                &json!({ "id": person_id }),
                Some("ResolveBrokerIds"),
                access_context,
                dpop_options,
            )
        },
    ) {
        if let Some(account_id) = response
            .get("account")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            context.account_id = account_id.to_string();
        }

        let mut portfolio_ids = response
            .get("account")
            .and_then(|v| v.get("brokerPortfolios"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .filter(|id| !id.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        portfolio_ids.sort();
        portfolio_ids.dedup();

        if portfolio_ids.len() == 1 {
            context.portfolio_id = portfolio_ids.into_iter().next();
        }
    }

    save_broker_context(context.clone())?;
    Ok(context)
}

fn context_account_or_session_person_id(
    session_manager: &SessionManager,
    env: TargetEnv,
) -> Result<String> {
    if let Some(existing) = load_broker_context()?
        && let Some(account_id) = Some(existing.account_id.trim()).filter(|v| !v.is_empty())
    {
        return Ok(account_id.to_string());
    }

    let stored = session_manager.load_required_active()?;
    if stored.env != env {
        bail!(
            "Stored session belongs to {}, not {env}. Run 'sc login' to replace it.",
            stored.env
        );
    }
    let session = stored.session;
    let account_id = session.person_id.trim();
    if account_id.is_empty() {
        bail!("Broker context invalid: session person id must be a non-empty string");
    }
    Ok(account_id.to_string())
}

pub(crate) fn run_broker_command_human(
    args: BrokerArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<HumanBrokerOutput> {
    match args.command {
        BrokerCommand::Context(context_args) => match context_args.command {
            BrokerContextCommand::Show(show_args) => {
                resolve_active_env(session_manager)?;
                let context = load_broker_context()?;
                let path = context_file_path()?;
                if show_args.json {
                    Ok(HumanBrokerOutput::Json(
                        json!({
                            "context_file": path,
                            "context": context,
                        }),
                        true,
                    ))
                } else {
                    let mut lines = vec![format!("context_file: {}", path.display())];
                    if let Some(ctx) = context {
                        lines.push(format!("account_id: {}", ctx.account_id));
                        lines.push(format!(
                            "portfolio_id: {}",
                            ctx.portfolio_id.unwrap_or_else(|| "<unset>".to_string())
                        ));
                    } else {
                        lines.push("account_id: <unset>".to_string());
                        lines.push("portfolio_id: <unset>".to_string());
                    }
                    Ok(HumanBrokerOutput::Text(lines))
                }
            }
            BrokerContextCommand::Select(select_args) => {
                let env = resolve_active_env(session_manager)?;
                let portfolio_id = select_args.portfolio_id.trim();
                if portfolio_id.is_empty() {
                    bail!("Broker context invalid: --portfolio-id must be a non-empty string");
                }
                let account_id = context_account_or_session_person_id(session_manager, env)?;
                let context = BrokerContext {
                    account_id,
                    portfolio_id: Some(portfolio_id.to_string()),
                };
                save_broker_context(context.clone())?;
                let payload = json!({
                    "context_file": context_file_path()?,
                    "context": context,
                    "saved": true,
                });
                if select_args.json {
                    Ok(HumanBrokerOutput::Json(payload, true))
                } else {
                    Ok(HumanBrokerOutput::Text(vec![
                        "Saved broker context.".to_string(),
                        format!("account_id: {}", context.account_id),
                        format!(
                            "portfolio_id: {}",
                            context.portfolio_id.as_deref().unwrap_or("<unset>")
                        ),
                    ]))
                }
            }
        },
        BrokerCommand::Overview(overview_args) => {
            let compact = overview_args.json;
            let payload = execute_broker_overview(overview_args, config, session_manager)?;
            Ok(HumanBrokerOutput::Json(payload, compact))
        }
        BrokerCommand::Analytics(analytics_args) => {
            let compact = analytics_args.json;
            let payload = execute_broker_analytics(analytics_args, config, session_manager)?;
            Ok(HumanBrokerOutput::Json(payload, compact))
        }
        BrokerCommand::Transactions(transactions_args) => {
            let compact = transactions_args.json;
            let payload = execute_broker_transactions(transactions_args, config, session_manager)?;
            Ok(HumanBrokerOutput::Json(payload, compact))
        }
        BrokerCommand::Transaction(transaction_args) => match transaction_args.command {
            BrokerTransactionCommand::Details(details_args) => {
                let compact = details_args.json;
                let payload =
                    execute_broker_transaction_details(details_args, config, session_manager)?;
                if compact {
                    Ok(HumanBrokerOutput::Json(payload, true))
                } else {
                    Ok(HumanBrokerOutput::Text(
                        render_broker_transaction_details_text(&payload),
                    ))
                }
            }
        },
        BrokerCommand::Holdings(holdings_args) => {
            let compact = holdings_args.json;
            let payload = execute_broker_holdings(holdings_args, config, session_manager)?;
            Ok(HumanBrokerOutput::Json(payload, compact))
        }
        BrokerCommand::Watchlist(watchlist_args) => {
            let crate::cli::BrokerWatchlistArgs {
                command,
                portfolio_id,
                include_year_to_date,
                quote_source,
                json,
            } = watchlist_args;
            match command {
                Some(BrokerWatchlistCommand::Add(add_args)) => {
                    let compact = add_args.json;
                    let payload = execute_broker_watchlist_add(add_args, config, session_manager)?;
                    Ok(HumanBrokerOutput::Json(payload, compact))
                }
                Some(BrokerWatchlistCommand::Remove(remove_args)) => {
                    let compact = remove_args.json;
                    let payload =
                        execute_broker_watchlist_remove(remove_args, config, session_manager)?;
                    Ok(HumanBrokerOutput::Json(payload, compact))
                }
                None => {
                    let payload = execute_broker_watchlist(
                        crate::cli::BrokerWatchlistArgs {
                            command: None,
                            portfolio_id,
                            include_year_to_date,
                            quote_source,
                            json,
                        },
                        config,
                        session_manager,
                    )?;
                    Ok(HumanBrokerOutput::Json(payload, json))
                }
            }
        }
        BrokerCommand::Search(search_args) => {
            let compact = search_args.json;
            let payload = execute_broker_search(search_args, config, session_manager)?;
            Ok(HumanBrokerOutput::Json(payload, compact))
        }
        BrokerCommand::Derivatives(derivatives_args) => match derivatives_args.command {
            BrokerDerivativesCommand::Search(search_args) => {
                let compact = search_args.json;
                let payload =
                    execute_broker_derivatives_search(search_args, config, session_manager)?;
                Ok(HumanBrokerOutput::Json(payload, compact))
            }
        },
        BrokerCommand::Quote(quote_args) => {
            let compact = quote_args.json;
            let payload = execute_broker_quote(quote_args, config, session_manager)?;
            Ok(HumanBrokerOutput::Json(payload, compact))
        }
        BrokerCommand::SecurityNews(news_args) => {
            let compact = news_args.json;
            let payload = execute_broker_security_news(news_args, config, session_manager)?;
            Ok(HumanBrokerOutput::Json(payload, compact))
        }
        BrokerCommand::PriceAlerts(price_alert_args) => {
            let crate::cli::BrokerPriceAlertsArgs {
                command,
                portfolio_id,
                active_only,
                json,
            } = price_alert_args;
            match command {
                Some(BrokerPriceAlertsCommand::Add(add_args)) => {
                    let compact = add_args.json;
                    let payload =
                        execute_broker_price_alert_add(add_args, config, session_manager)?;
                    Ok(HumanBrokerOutput::Json(payload, compact))
                }
                Some(BrokerPriceAlertsCommand::Remove(remove_args)) => {
                    let compact = remove_args.json;
                    let payload =
                        execute_broker_price_alert_remove(remove_args, config, session_manager)?;
                    Ok(HumanBrokerOutput::Json(payload, compact))
                }
                None => {
                    let payload = execute_broker_price_alerts(
                        crate::cli::BrokerPriceAlertsArgs {
                            command: None,
                            portfolio_id,
                            active_only,
                            json,
                        },
                        config,
                        session_manager,
                    )?;
                    Ok(HumanBrokerOutput::Json(payload, json))
                }
            }
        }
        BrokerCommand::SavingsPlans(savings_plans_args) => {
            let crate::cli::BrokerSavingsPlansArgs {
                command,
                portfolio_id,
                json,
            } = savings_plans_args;
            match command {
                Some(BrokerSavingsPlansCommand::Add(add_args)) => {
                    let compact = add_args.json;
                    let payload =
                        execute_broker_savings_plan_add(add_args, config, session_manager)?;
                    Ok(HumanBrokerOutput::Json(payload, compact))
                }
                Some(BrokerSavingsPlansCommand::Remove(remove_args)) => {
                    let compact = remove_args.json;
                    let payload =
                        execute_broker_savings_plan_remove(remove_args, config, session_manager)?;
                    Ok(HumanBrokerOutput::Json(payload, compact))
                }
                None => {
                    let payload = execute_broker_savings_plans(
                        crate::cli::BrokerSavingsPlansArgs {
                            command: None,
                            portfolio_id,
                            json,
                        },
                        config,
                        session_manager,
                    )?;
                    Ok(HumanBrokerOutput::Json(payload, json))
                }
            }
        }
        BrokerCommand::Trade(trade_args) => match trade_args.command {
            BrokerTradeCommand::Buy(buy_args) => {
                let compact = buy_args.json;
                let payload = execute_broker_trade_buy(buy_args, config, session_manager)?;
                if compact {
                    Ok(HumanBrokerOutput::Json(payload, true))
                } else {
                    Ok(HumanBrokerOutput::Text(render_trade_buy_text(&payload)))
                }
            }
            BrokerTradeCommand::Sell(sell_args) => {
                let compact = sell_args.json;
                let payload = execute_broker_trade_sell(sell_args, config, session_manager)?;
                if compact {
                    Ok(HumanBrokerOutput::Json(payload, true))
                } else {
                    Ok(HumanBrokerOutput::Text(render_trade_sell_text(&payload)))
                }
            }
            BrokerTradeCommand::Cancel(cancel_args) => {
                let compact = cancel_args.json;
                let payload = execute_broker_trade_cancel(cancel_args, config, session_manager)?;
                if compact {
                    Ok(HumanBrokerOutput::Json(payload, true))
                } else {
                    Ok(HumanBrokerOutput::Text(render_trade_cancel_text(&payload)))
                }
            }
        },
    }
}

pub(crate) fn run_broker_command_machine(
    args: BrokerArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    match args.command {
        BrokerCommand::Context(context_args) => match context_args.command {
            BrokerContextCommand::Show(_show_args) => {
                let _env = resolve_active_env(session_manager)?;
                Ok(json!({
                    "context_file": context_file_path()?,
                    "context": load_broker_context()?,
                }))
            }
            BrokerContextCommand::Select(select_args) => {
                let env = resolve_active_env(session_manager)?;
                let portfolio_id = select_args.portfolio_id.trim();
                if portfolio_id.is_empty() {
                    bail!("Broker context invalid: --portfolio-id must be a non-empty string");
                }
                let account_id = context_account_or_session_person_id(session_manager, env)?;
                let context = BrokerContext {
                    account_id,
                    portfolio_id: Some(portfolio_id.to_string()),
                };
                save_broker_context(context.clone())?;
                Ok(json!({
                    "context_file": context_file_path()?,
                    "context": context,
                    "saved": true,
                }))
            }
        },
        BrokerCommand::Overview(args) => execute_broker_overview(args, config, session_manager),
        BrokerCommand::Analytics(args) => execute_broker_analytics(args, config, session_manager),
        BrokerCommand::Transactions(args) => {
            execute_broker_transactions(args, config, session_manager)
        }
        BrokerCommand::Transaction(transaction_args) => match transaction_args.command {
            BrokerTransactionCommand::Details(args) => {
                execute_broker_transaction_details(args, config, session_manager)
            }
        },
        BrokerCommand::Holdings(args) => execute_broker_holdings(args, config, session_manager),
        BrokerCommand::Watchlist(args) => {
            let crate::cli::BrokerWatchlistArgs {
                command,
                portfolio_id,
                include_year_to_date,
                quote_source,
                json,
            } = args;
            match command {
                Some(BrokerWatchlistCommand::Add(add_args)) => {
                    execute_broker_watchlist_add(add_args, config, session_manager)
                }
                Some(BrokerWatchlistCommand::Remove(remove_args)) => {
                    execute_broker_watchlist_remove(remove_args, config, session_manager)
                }
                None => execute_broker_watchlist(
                    crate::cli::BrokerWatchlistArgs {
                        command: None,
                        portfolio_id,
                        include_year_to_date,
                        quote_source,
                        json,
                    },
                    config,
                    session_manager,
                ),
            }
        }
        BrokerCommand::Search(args) => execute_broker_search(args, config, session_manager),
        BrokerCommand::Derivatives(args) => match args.command {
            BrokerDerivativesCommand::Search(search_args) => {
                execute_broker_derivatives_search(search_args, config, session_manager)
            }
        },
        BrokerCommand::Quote(args) => execute_broker_quote(args, config, session_manager),
        BrokerCommand::SecurityNews(args) => {
            execute_broker_security_news(args, config, session_manager)
        }
        BrokerCommand::PriceAlerts(args) => {
            let crate::cli::BrokerPriceAlertsArgs {
                command,
                portfolio_id,
                active_only,
                json,
            } = args;
            match command {
                Some(BrokerPriceAlertsCommand::Add(add_args)) => {
                    execute_broker_price_alert_add(add_args, config, session_manager)
                }
                Some(BrokerPriceAlertsCommand::Remove(remove_args)) => {
                    execute_broker_price_alert_remove(remove_args, config, session_manager)
                }
                None => execute_broker_price_alerts(
                    crate::cli::BrokerPriceAlertsArgs {
                        command: None,
                        portfolio_id,
                        active_only,
                        json,
                    },
                    config,
                    session_manager,
                ),
            }
        }
        BrokerCommand::SavingsPlans(args) => {
            let crate::cli::BrokerSavingsPlansArgs {
                command,
                portfolio_id,
                json,
            } = args;
            match command {
                Some(BrokerSavingsPlansCommand::Add(add_args)) => {
                    execute_broker_savings_plan_add(add_args, config, session_manager)
                }
                Some(BrokerSavingsPlansCommand::Remove(remove_args)) => {
                    execute_broker_savings_plan_remove(remove_args, config, session_manager)
                }
                None => execute_broker_savings_plans(
                    crate::cli::BrokerSavingsPlansArgs {
                        command: None,
                        portfolio_id,
                        json,
                    },
                    config,
                    session_manager,
                ),
            }
        }
        BrokerCommand::Trade(trade_args) => match trade_args.command {
            BrokerTradeCommand::Buy(buy_args) => {
                execute_broker_trade_buy(buy_args, config, session_manager)
            }
            BrokerTradeCommand::Sell(sell_args) => {
                execute_broker_trade_sell(sell_args, config, session_manager)
            }
            BrokerTradeCommand::Cancel(cancel_args) => {
                execute_broker_trade_cancel(cancel_args, config, session_manager)
            }
        },
    }
}

pub(crate) fn execute_broker_overview(
    args: crate::cli::BrokerOverviewArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_overview_query(args, config, session_manager)
}

pub(crate) fn execute_broker_analytics(
    args: crate::cli::BrokerAnalyticsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_analytics_query(args, config, session_manager)
}

pub(crate) fn execute_broker_transactions(
    args: crate::cli::BrokerTransactionsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_transactions_query(args, config, session_manager)
}

pub(crate) fn execute_broker_transaction_details(
    args: crate::cli::BrokerTransactionDetailsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_transaction_details_query(args, config, session_manager)
}

pub(crate) fn execute_broker_holdings(
    args: crate::cli::BrokerHoldingsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_holdings_query(args, config, session_manager)
}

fn render_broker_transaction_details_text(payload: &Value) -> Vec<String> {
    let result = payload.get("result").unwrap_or(payload);
    let currency = result.get("currency").and_then(Value::as_str);
    let detail_type = result.get("detail_type").and_then(Value::as_str);

    let mut lines = vec![
        format!("id: {}", display_value(result.get("id"))),
        format!(
            "transaction_reference: {}",
            display_value(result.get("transaction_reference"))
        ),
        format!("type: {}", display_value(result.get("type"))),
        format!("detail_type: {}", display_value(result.get("detail_type"))),
        format!("currency: {}", display_value(result.get("currency"))),
        format!(
            "last_event_datetime: {}",
            display_value(result.get("last_event_datetime"))
        ),
    ];

    if let Some(security) = result.get("security").filter(|value| !value.is_null()) {
        lines.push(format!(
            "security_isin: {}",
            display_value(security.get("isin"))
        ));
        lines.push(format!(
            "security_name: {}",
            display_value(security.get("name"))
        ));
        lines.push(format!(
            "security_type: {}",
            display_value(security.get("security_type"))
        ));
    }

    match detail_type {
        Some("security_trade") => render_security_trade_text(&mut lines, result, currency),
        Some("cash") => render_cash_transaction_text(&mut lines, result, currency),
        Some("non_trade_security") => {
            render_non_trade_transaction_text(&mut lines, result, currency)
        }
        Some("eltif") => render_eltif_transaction_text(&mut lines, result, currency),
        _ => {}
    }

    let document_count = result
        .get("documents")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    lines.push(format!("documents: {document_count}"));

    let linked_ids = result
        .get("linked_transaction_ids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    lines.push(format!(
        "linked_transaction_ids: {}",
        if linked_ids.is_empty() {
            "<none>".to_string()
        } else {
            linked_ids.join(", ")
        }
    ));

    let history_count = result
        .get("history")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    lines.push(format!("history_entries: {history_count}"));

    lines
}

fn render_security_trade_text(lines: &mut Vec<String>, result: &Value, currency: Option<&str>) {
    let security_trade = result
        .get("security_trade")
        .filter(|value| !value.is_null())
        .unwrap_or(&Value::Null);
    let shares = security_trade
        .get("number_of_shares")
        .unwrap_or(&Value::Null);
    let trade_amounts = security_trade
        .get("trade_transaction_amounts")
        .unwrap_or(&Value::Null);
    let aggregated_taxes = security_trade
        .get("aggregated_transaction_taxes")
        .unwrap_or(&Value::Null);

    lines.push(format!(
        "status: {}",
        display_value(security_trade.get("status"))
    ));
    lines.push(format!(
        "side: {}",
        display_value(security_trade.get("side"))
    ));
    lines.push(format!(
        "order_kind: {}",
        display_value(security_trade.get("order_kind"))
    ));
    lines.push(format!(
        "quantity_filled: {}",
        display_value(shares.get("filled"))
    ));
    lines.push(format!(
        "quantity_total: {}",
        display_value(shares.get("total"))
    ));
    lines.push(format!(
        "average_price: {}",
        display_money(security_trade.get("average_price"), currency)
    ));
    lines.push(format!(
        "total_amount: {}",
        display_money(security_trade.get("total_amount"), currency)
    ));
    lines.push(format!(
        "finalisation_reason: {}",
        display_value(security_trade.get("finalisation_reason"))
    ));
    lines.push(format!(
        "limit_price: {}",
        display_money(security_trade.get("limit_price"), currency)
    ));
    lines.push(format!(
        "stop_price: {}",
        display_money(security_trade.get("stop_price"), currency)
    ));
    lines.push(format!(
        "valid_until: {}",
        display_value(security_trade.get("valid_until"))
    ));
    lines.push(format!(
        "is_cancellation_requested: {}",
        display_value(security_trade.get("is_cancellation_requested"))
    ));
    lines.push(format!(
        "trading_venue: {}",
        display_value(security_trade.get("trading_venue"))
    ));
    lines.push(format!(
        "fee: {}",
        display_money(security_trade.get("fee"), currency)
    ));
    lines.push(format!(
        "transactional_fee: {}",
        display_money(security_trade.get("transactional_fee"), currency)
    ));
    lines.push(format!(
        "taxes: {}",
        display_money(security_trade.get("taxes"), currency)
    ));
    lines.push(format!(
        "trade_tax_amount: {}",
        display_money(trade_amounts.get("tax_amount"), currency)
    ));
    lines.push(format!(
        "transaction_fee: {}",
        display_money(trade_amounts.get("transaction_fee"), currency)
    ));
    lines.push(format!(
        "venue_fee: {}",
        display_money(trade_amounts.get("venue_fee"), currency)
    ));
    lines.push(format!(
        "crypto_spread_fee: {}",
        display_money(trade_amounts.get("crypto_spread_fee"), currency)
    ));
    lines.push(format!(
        "total_tax: {}",
        display_money(aggregated_taxes.get("total_tax"), currency)
    ));
    lines.push(format!(
        "capital_gains_tax: {}",
        display_money(aggregated_taxes.get("capital_gains_tax"), currency)
    ));
    lines.push(format!(
        "church_tax: {}",
        display_money(aggregated_taxes.get("church_tax"), currency)
    ));
    lines.push(format!(
        "solidarity_tax: {}",
        display_money(aggregated_taxes.get("solidarity_tax"), currency)
    ));
    lines.push(format!(
        "source_tax: {}",
        display_money(aggregated_taxes.get("source_tax"), currency)
    ));
    lines.push(format!(
        "financial_transaction_tax: {}",
        display_money(aggregated_taxes.get("financial_transaction_tax"), currency)
    ));
}

fn render_cash_transaction_text(lines: &mut Vec<String>, result: &Value, currency: Option<&str>) {
    let cash = result
        .get("cash")
        .filter(|value| !value.is_null())
        .unwrap_or(&Value::Null);
    let tax_details = cash.get("tax_details").unwrap_or(&Value::Null);
    let sddi_details = cash.get("sddi_details").unwrap_or(&Value::Null);

    lines.push(format!(
        "cash_transaction_type: {}",
        display_value(cash.get("cash_transaction_type"))
    ));
    lines.push(format!(
        "amount: {}",
        display_money(cash.get("amount"), currency)
    ));
    lines.push(format!(
        "description: {}",
        display_value(cash.get("description"))
    ));
    lines.push(format!(
        "tax_gross_amount: {}",
        display_money(tax_details.get("gross_amount"), currency)
    ));
    lines.push(format!(
        "tax_amount: {}",
        display_money(tax_details.get("tax_amount"), currency)
    ));
    lines.push(format!(
        "sddi_fee: {}",
        display_money(sddi_details.get("fee"), currency)
    ));
    lines.push(format!(
        "sddi_gross_amount: {}",
        display_money(sddi_details.get("gross_amount"), currency)
    ));
}

fn render_non_trade_transaction_text(
    lines: &mut Vec<String>,
    result: &Value,
    currency: Option<&str>,
) {
    let non_trade = result
        .get("non_trade_security")
        .filter(|value| !value.is_null())
        .unwrap_or(&Value::Null);

    lines.push(format!("isin: {}", display_value(non_trade.get("isin"))));
    lines.push(format!(
        "non_trade_security_transaction_type: {}",
        display_value(non_trade.get("non_trade_security_transaction_type"))
    ));
    lines.push(format!(
        "quantity: {}",
        display_value(non_trade.get("quantity"))
    ));
    lines.push(format!(
        "average_price: {}",
        display_money(non_trade.get("average_price"), currency)
    ));
    lines.push(format!(
        "total_amount: {}",
        display_money(non_trade.get("total_amount"), currency)
    ));
    lines.push(format!(
        "description: {}",
        display_value(non_trade.get("description"))
    ));
}

fn render_eltif_transaction_text(lines: &mut Vec<String>, result: &Value, currency: Option<&str>) {
    let eltif = result
        .get("eltif")
        .filter(|value| !value.is_null())
        .unwrap_or(&Value::Null);
    let cancelable = eltif.get("cancelable_details").unwrap_or(&Value::Null);

    lines.push(format!("status: {}", display_value(eltif.get("status"))));
    lines.push(format!("side: {}", display_value(eltif.get("side"))));
    lines.push(format!(
        "order_kind: {}",
        display_value(eltif.get("order_kind"))
    ));
    lines.push(format!(
        "amount: {}",
        display_money(eltif.get("amount"), currency)
    ));
    lines.push(format!(
        "eltif_quantity: {}",
        display_value(eltif.get("eltif_quantity"))
    ));
    lines.push(format!(
        "execution_price: {}",
        display_money(eltif.get("execution_price"), currency)
    ));
    lines.push(format!(
        "execution_date: {}",
        display_value(eltif.get("execution_date"))
    ));
    lines.push(format!(
        "earliest_sell_date: {}",
        display_value(eltif.get("earliest_sell_date"))
    ));
    lines.push(format!(
        "market_valuation: {}",
        display_money(eltif.get("market_valuation"), currency)
    ));
    lines.push(format!(
        "finalisation_reason: {}",
        display_value(eltif.get("finalisation_reason"))
    ));
    lines.push(format!(
        "trading_venue: {}",
        display_value(eltif.get("trading_venue"))
    ));
    lines.push(format!(
        "is_multiple_orders_cancellation: {}",
        display_value(eltif.get("is_multiple_orders_cancellation"))
    ));
    lines.push(format!(
        "is_initial_investment: {}",
        display_value(eltif.get("is_initial_investment"))
    ));
    lines.push(format!(
        "cancelable_days_left: {}",
        display_value(cancelable.get("days_left"))
    ));
    lines.push(format!(
        "is_cancelable: {}",
        display_value(cancelable.get("is_cancelable"))
    ));
}

fn display_value(value: Option<&Value>) -> String {
    match value.unwrap_or(&Value::Null) {
        Value::Null => "<none>".to_string(),
        Value::Bool(raw) => raw.to_string(),
        Value::String(raw) => raw.clone(),
        other => other.to_string(),
    }
}

fn display_money(value: Option<&Value>, currency: Option<&str>) -> String {
    match value.unwrap_or(&Value::Null) {
        Value::Null => "<none>".to_string(),
        Value::String(raw) => match currency.filter(|currency| !currency.is_empty()) {
            Some(currency) => format!("{raw} {currency}"),
            None => raw.clone(),
        },
        other => match currency.filter(|currency| !currency.is_empty()) {
            Some(currency) => format!("{other} {currency}"),
            None => other.to_string(),
        },
    }
}

pub(crate) fn execute_broker_watchlist(
    args: crate::cli::BrokerWatchlistArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_watchlist_query(args, config, session_manager)
}

pub(crate) fn execute_broker_watchlist_add(
    args: crate::cli::BrokerWatchlistAddArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let mut session = loaded.session;
    let access_context = loaded.access_context;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let requested_isin = args.isin.trim().to_string();
    let variables = broker_add_to_watchlist_variables(&ids.portfolio_id, &requested_isin)?;
    let response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                BROKER_ADD_TO_WATCHLIST_MUTATION,
                &variables,
                Some("BrokerAddToWatchlist"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_watchlist_add_response(&requested_isin, &response)?;
    Ok(json!({
        "account_id": ids.account_id,
        "portfolio_id": ids.portfolio_id,
        "resolution": {
            "account": ids.account_source,
            "portfolio": ids.portfolio_source,
        },
        "result": projected,
    }))
}

pub(crate) fn execute_broker_watchlist_remove(
    args: crate::cli::BrokerWatchlistRemoveArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let mut session = loaded.session;
    let access_context = loaded.access_context;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let requested_isin = args.isin.trim().to_string();
    let variables = broker_remove_from_watchlist_variables(&ids.portfolio_id, &requested_isin)?;
    let response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                BROKER_REMOVE_FROM_WATCHLIST_MUTATION,
                &variables,
                Some("BrokerRemoveFromWatchlist"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_watchlist_remove_response(&requested_isin, &response)?;
    Ok(json!({
        "account_id": ids.account_id,
        "portfolio_id": ids.portfolio_id,
        "resolution": {
            "account": ids.account_source,
            "portfolio": ids.portfolio_source,
        },
        "result": projected,
    }))
}

pub(crate) fn execute_broker_search(
    args: crate::cli::BrokerSearchArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_search_query(args, config, session_manager)
}

pub(crate) fn execute_broker_derivatives_search(
    args: crate::cli::BrokerDerivativesSearchArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_derivatives_search_query(args, config, session_manager)
}

pub(crate) fn execute_broker_quote(
    args: crate::cli::BrokerQuoteArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_quote_query(args, config, session_manager)
}

pub(crate) fn execute_broker_security_news(
    args: crate::cli::BrokerSecurityNewsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_security_news_query(args, config, session_manager)
}

pub(crate) fn execute_broker_price_alerts(
    args: crate::cli::BrokerPriceAlertsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_price_alerts_query(args, config, session_manager)
}

pub(crate) fn execute_broker_price_alert_add(
    args: crate::cli::BrokerPriceAlertAddArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let mut session = loaded.session;
    let access_context = loaded.access_context;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, false, None)?;

    let requested_price = args.price.trim().to_string();

    let projected = if let Some(isin) = args
        .isin
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let variables =
            broker_add_price_alert_variables(&ids.portfolio_id, isin, requested_price.as_str())?;
        let response = execute_with_refresh_retry(
            session_manager,
            env,
            &env_cfg,
            &mut session,
            dpop_options,
            |token| {
                execute_graphql(
                    &env_cfg.graphql_url,
                    token,
                    BROKER_ADD_PRICE_ALERT_MUTATION,
                    &variables,
                    Some("BrokerAddPriceAlert"),
                    access_context,
                    dpop_options,
                )
            },
        )?;
        project_broker_add_price_alert_response(&input, isin, requested_price.as_str(), &response)?
    } else if let Some(ticker) = args
        .ticker
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let variables = broker_add_crypto_price_alert_variables(
            &ids.portfolio_id,
            ticker,
            requested_price.as_str(),
        )?;
        let response = execute_with_refresh_retry(
            session_manager,
            env,
            &env_cfg,
            &mut session,
            dpop_options,
            |token| {
                execute_graphql(
                    &env_cfg.graphql_url,
                    token,
                    BROKER_ADD_CRYPTO_PRICE_ALERT_MUTATION,
                    &variables,
                    Some("BrokerAddCryptoPriceAlert"),
                    access_context,
                    dpop_options,
                )
            },
        )?;
        project_broker_add_crypto_price_alert_response(
            &input,
            ticker,
            requested_price.as_str(),
            &response,
        )?
    } else {
        bail!("Provide exactly one of --isin or --ticker");
    };

    Ok(json!({
        "account_id": ids.account_id,
        "portfolio_id": ids.portfolio_id,
        "resolution": {
            "account": ids.account_source,
            "portfolio": ids.portfolio_source,
        },
        "result": projected,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedPriceAlert {
    Security { alert_id: String, isin: String },
    Crypto { alert_id: String, ticker: String },
}

pub(crate) fn execute_broker_price_alert_remove(
    args: crate::cli::BrokerPriceAlertRemoveArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let mut session = loaded.session;
    let access_context = loaded.access_context;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, false, None)?;
    let requested_alert_id = args.alert_id.trim().to_string();

    let resolved = lookup_price_alert_by_id(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        &input,
        requested_alert_id.as_str(),
    )?;

    let projected = match resolved {
        ResolvedPriceAlert::Security { alert_id, isin } => {
            let variables = broker_remove_price_alert_variables(&ids.portfolio_id, &alert_id)?;
            let response = execute_with_refresh_retry(
                session_manager,
                env,
                &env_cfg,
                &mut session,
                dpop_options,
                |token| {
                    execute_graphql(
                        &env_cfg.graphql_url,
                        token,
                        BROKER_REMOVE_PRICE_ALERT_MUTATION,
                        &variables,
                        Some("BrokerRemovePriceAlert"),
                        access_context,
                        dpop_options,
                    )
                },
            )?;
            project_broker_remove_price_alert_response(&alert_id, &isin, &response)?
        }
        ResolvedPriceAlert::Crypto { alert_id, ticker } => {
            let variables = broker_remove_price_alert_variables(&ids.portfolio_id, &alert_id)?;
            let response = execute_with_refresh_retry(
                session_manager,
                env,
                &env_cfg,
                &mut session,
                dpop_options,
                |token| {
                    execute_graphql(
                        &env_cfg.graphql_url,
                        token,
                        BROKER_REMOVE_CRYPTO_PRICE_ALERT_MUTATION,
                        &variables,
                        Some("BrokerRemoveCryptoPriceAlert"),
                        access_context,
                        dpop_options,
                    )
                },
            )?;
            project_broker_remove_crypto_price_alert_response(&alert_id, &ticker, &response)?
        }
    };

    Ok(json!({
        "account_id": ids.account_id,
        "portfolio_id": ids.portfolio_id,
        "resolution": {
            "account": ids.account_source,
            "portfolio": ids.portfolio_source,
        },
        "result": projected,
    }))
}

fn lookup_price_alert_by_id(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    session: &mut Session,
    dpop_options: &crate::dpop::DpopRuntimeOptions,
    input: &crate::helpers::BrokerInput,
    requested_alert_id: &str,
) -> Result<ResolvedPriceAlert> {
    let requested_alert_id = requested_alert_id.trim();
    if requested_alert_id.is_empty() {
        bail!("Broker input invalid: field 'alert_id' must be a non-empty string");
    }

    let security_variables = crate::helpers::broker_price_alerts_variables(input, false)?;
    let security_response = execute_with_refresh_retry(
        session_manager,
        env,
        env_cfg,
        session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                BROKER_PRICE_ALERTS_QUERY,
                &security_variables,
                Some("BrokerPriceAlerts"),
                crate::graphql::GraphqlAccessContext::default(),
                dpop_options,
            )
        },
    )?;
    let security_projected =
        project_broker_security_price_alerts_response(input, &security_response)?;

    let security_items = security_projected
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing projected security price-alert items")
        })?
        .to_vec();

    resolve_price_alert_lookup_with_crypto_loader(requested_alert_id, &security_items, || {
        let crypto_variables = broker_crypto_price_alerts_variables(input)?;
        let crypto_response = execute_with_refresh_retry(
            session_manager,
            env,
            env_cfg,
            session,
            dpop_options,
            |token| {
                execute_graphql(
                    &env_cfg.graphql_url,
                    token,
                    BROKER_CRYPTO_PRICE_ALERTS_QUERY,
                    &crypto_variables,
                    Some("BrokerCryptoPriceAlerts"),
                    crate::graphql::GraphqlAccessContext::default(),
                    dpop_options,
                )
            },
        )?;
        let crypto_projected =
            project_broker_crypto_price_alerts_response(input, &crypto_response)?;
        let crypto_items = crypto_projected
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow!("Broker response invalid: missing projected crypto price-alert items")
            })?
            .to_vec();

        Ok(crypto_items)
    })
}

fn project_broker_security_price_alerts_response(
    input: &crate::helpers::BrokerInput,
    response: &Value,
) -> Result<Value> {
    crate::helpers::project_broker_price_alerts_response(input, false, response)
}

fn resolve_price_alert_lookup_with_crypto_loader<F>(
    requested_alert_id: &str,
    security_items: &[Value],
    load_crypto_items: F,
) -> Result<ResolvedPriceAlert>
where
    F: FnOnce() -> Result<Vec<Value>>,
{
    let requested_alert_id = requested_alert_id.trim();
    if requested_alert_id.is_empty() {
        bail!("Broker input invalid: field 'alert_id' must be a non-empty string");
    }

    if let Some(alert) = find_security_price_alert_match(security_items, requested_alert_id)? {
        return Ok(alert);
    }

    let crypto_items = load_crypto_items()?;
    resolve_price_alert_lookup_from_items(requested_alert_id, security_items, &crypto_items)
}

fn resolve_price_alert_lookup_from_items(
    requested_alert_id: &str,
    security_items: &[Value],
    crypto_items: &[Value],
) -> Result<ResolvedPriceAlert> {
    let requested_alert_id = requested_alert_id.trim();
    if requested_alert_id.is_empty() {
        bail!("Broker input invalid: field 'alert_id' must be a non-empty string");
    }

    let security_match = find_security_price_alert_match(security_items, requested_alert_id)?;
    let crypto_match = find_crypto_price_alert_match(crypto_items, requested_alert_id)?;

    match (security_match, crypto_match) {
        (Some(_), Some(_)) => Err(anyhow!(
            "Broker response invalid: price alert '{requested_alert_id}' matched multiple alert kinds in the active portfolio"
        )),
        (Some(alert), None) => Ok(alert),
        (None, Some(alert)) => Ok(alert),
        (None, None) => bail!(
            "Broker input invalid: price alert '{requested_alert_id}' was not found in the active portfolio"
        ),
    }
}

fn find_security_price_alert_match(
    items: &[Value],
    requested_alert_id: &str,
) -> Result<Option<ResolvedPriceAlert>> {
    let mut matches = items
        .iter()
        .filter(|item| item.get("alert_id").and_then(Value::as_str) == Some(requested_alert_id));

    let first = match matches.next() {
        Some(item) => item,
        None => return Ok(None),
    };

    if matches.next().is_some() {
        return Err(anyhow!(
            "Broker response invalid: price alert '{requested_alert_id}' matched multiple security alerts in the active portfolio"
        ));
    }

    let isin = first
        .get("isin")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "Broker response invalid: security price alert '{requested_alert_id}' is missing isin"
            )
        })?;

    Ok(Some(ResolvedPriceAlert::Security {
        alert_id: requested_alert_id.to_string(),
        isin: isin.to_string(),
    }))
}

fn find_crypto_price_alert_match(
    items: &[Value],
    requested_alert_id: &str,
) -> Result<Option<ResolvedPriceAlert>> {
    let mut matches = items
        .iter()
        .filter(|item| item.get("alert_id").and_then(Value::as_str) == Some(requested_alert_id));

    let first = match matches.next() {
        Some(item) => item,
        None => return Ok(None),
    };

    if matches.next().is_some() {
        return Err(anyhow!(
            "Broker response invalid: price alert '{requested_alert_id}' matched multiple crypto alerts in the active portfolio"
        ));
    }

    let ticker = first
        .get("ticker")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "Broker response invalid: crypto price alert '{requested_alert_id}' is missing ticker"
            )
        })?;

    Ok(Some(ResolvedPriceAlert::Crypto {
        alert_id: requested_alert_id.to_string(),
        ticker: ticker.to_string(),
    }))
}

#[allow(dead_code)]
pub(crate) fn execute_broker_limits(
    args: crate::cli::BrokerLimitsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    crate::broker_query_execution::execute_broker_limits(args, config, session_manager)
}

pub(crate) fn execute_broker_savings_plans(
    args: crate::cli::BrokerSavingsPlansArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_savings_plans_query(args, config, session_manager)
}

pub(crate) fn execute_broker_savings_plan_add(
    args: crate::cli::BrokerSavingsPlanAddArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let mut session = loaded.session;
    let access_context = loaded.access_context;

    let isin = args.isin.trim().to_string();
    if isin.is_empty() {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field 'isin' must be a non-empty string");
    }

    let amount = normalize_positive_decimal_for_savings(args.amount.as_str(), "amount")?;
    let amount_value = parse_positive_decimal_for_savings(amount.as_str(), "amount")?;

    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, false, None)?;

    let config_variables = broker_savings_plan_config_variables(&input, &isin)
        .map_err(|err| anyhow!("SAVINGS_PLAN_INPUT_INVALID: {err}"))?;
    let config_response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                BROKER_SAVINGS_PLAN_CONFIG_QUERY,
                &config_variables,
                Some("BrokerSavingsPlanConfig"),
                access_context,
                dpop_options,
            )
        },
    )?;

    let config_projected = project_broker_savings_plan_config_response(&config_response).map_err(
        |_| {
            anyhow!(
                "SAVINGS_PLAN_CONFIG_UNAVAILABLE: savings plan is not available for this instrument in the selected portfolio context"
            )
        },
    )?;
    let parsed_config = parse_broker_savings_plan_config(&config_projected).map_err(|_| {
        anyhow!(
            "SAVINGS_PLAN_CONFIG_UNAVAILABLE: savings plan is not available for this instrument in the selected portfolio context"
        )
    })?;
    validate_amount_against_config(amount_value, &parsed_config)?;
    let effective_config = resolve_effective_savings_plan_add_config(&args, &parsed_config)?;

    let mutation_variables = broker_create_or_update_savings_plan_variables(
        &ids.portfolio_id,
        &isin,
        &amount,
        effective_config.frequency.as_str(),
        effective_config.day_of_month,
        effective_config.year_month.as_str(),
        effective_config.dynamization_rate.as_str(),
        effective_config.payment_method.as_str(),
        args.appropriateness_id.as_deref(),
        args.acknowledged_appropriateness_warning_version.as_deref(),
    )
    .map_err(|err| anyhow!("SAVINGS_PLAN_INPUT_INVALID: {err}"))?;

    let mutation_response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                BROKER_CREATE_OR_UPDATE_SAVINGS_PLAN_MUTATION,
                &mutation_variables,
                Some("BrokerCreateOrUpdateSavingsPlan"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let mutation_projected =
        project_broker_create_or_update_savings_plan_response(&mutation_response)?;

    let readback_variables = broker_savings_plan_by_isin_variables(&input, &isin)
        .map_err(|err| anyhow!("SAVINGS_PLAN_INPUT_INVALID: {err}"))?;
    let readback_response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                BROKER_SAVINGS_PLAN_BY_ISIN_QUERY,
                &readback_variables,
                Some("BrokerSavingsPlanByIsin"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let readback_projected = project_broker_savings_plan_by_isin_response(&readback_response)?;
    let savings_plan = readback_projected
        .get("savings_plan")
        .cloned()
        .unwrap_or(Value::Null);

    Ok(json!({
        "account_id": ids.account_id,
        "portfolio_id": ids.portfolio_id,
        "resolution": {
            "account": ids.account_source,
            "portfolio": ids.portfolio_source,
        },
        "result": {
            "action": "create_or_update",
            "security": readback_projected
                .get("security")
                .cloned()
                .unwrap_or(Value::Null),
            "input": {
                "isin": isin,
                "amount": amount,
                "frequency": args.frequency.map(|v| v.as_graphql()),
                "day_of_month": args.day_of_month,
                "year_month": args.year_month.as_deref(),
                "dynamization_rate": args.dynamization_rate.as_deref(),
                "payment_method": args.payment_method.map(|v| v.as_graphql()),
                "appropriateness_id": args.appropriateness_id.as_deref(),
                "acknowledged_appropriateness_warning_version": args.acknowledged_appropriateness_warning_version.as_deref(),
            },
            "effective_configuration": {
                "frequency": effective_config.frequency,
                "day_of_month": effective_config.day_of_month,
                "year_month": effective_config.year_month,
                "dynamization_rate": effective_config.dynamization_rate,
                "payment_method": effective_config.payment_method,
            },
            "mutation_id": mutation_projected
                .get("mutation_id")
                .cloned()
                .unwrap_or(Value::Null),
            "savings_plan": savings_plan.clone(),
            "warning": if savings_plan.is_null() {
                Value::String("mutation completed but no active savings plan returned in readback".to_string())
            } else {
                Value::Null
            },
        },
    }))
}

pub(crate) fn execute_broker_savings_plan_remove(
    args: crate::cli::BrokerSavingsPlanRemoveArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let mut session = loaded.session;
    let access_context = loaded.access_context;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;

    let requested_isin = args.isin.trim().to_string();
    if requested_isin.is_empty() {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field 'isin' must be a non-empty string");
    }

    let variables = broker_remove_savings_plan_variables(&ids.portfolio_id, &requested_isin)
        .map_err(|err| anyhow!("SAVINGS_PLAN_INPUT_INVALID: {err}"))?;
    let response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                BROKER_REMOVE_SAVINGS_PLAN_MUTATION,
                &variables,
                Some("BrokerRemoveSavingsPlan"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_remove_savings_plan_response(&requested_isin, &response)?;

    Ok(json!({
        "account_id": ids.account_id,
        "portfolio_id": ids.portfolio_id,
        "resolution": {
            "account": ids.account_source,
            "portfolio": ids.portfolio_source,
        },
        "result": projected,
    }))
}

#[derive(Debug, Clone)]
struct ParsedBrokerSavingsPlanConfig {
    min_amount: f64,
    max_amount: f64,
    frequencies: Vec<String>,
    payment_methods: Vec<String>,
    dynamization_rates: Vec<String>,
    default_dynamization_rate: String,
    schedules: Vec<ParsedBrokerSavingsPlanSchedule>,
}

#[derive(Debug, Clone)]
struct ParsedBrokerSavingsPlanSchedule {
    day_of_month: u8,
    is_default: bool,
    is_earliest: bool,
    available_year_months: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResolvedSavingsPlanAddConfig {
    frequency: String,
    day_of_month: u8,
    year_month: String,
    dynamization_rate: String,
    payment_method: String,
}

fn normalize_positive_decimal_for_savings(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field '{field}' must be a positive decimal");
    }
    let dot_count = trimmed.chars().filter(|c| *c == '.').count();
    let has_only_decimal_chars = trimmed.chars().all(|c| c.is_ascii_digit() || c == '.');
    if !has_only_decimal_chars || dot_count > 1 {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field '{field}' must be a positive decimal");
    }
    let parsed = trimmed.parse::<f64>().map_err(|_| {
        anyhow!("SAVINGS_PLAN_INPUT_INVALID: field '{field}' must be a positive decimal")
    })?;
    if !parsed.is_finite() || parsed <= 0.0 {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field '{field}' must be a positive decimal");
    }
    Ok(trimmed.to_string())
}

fn parse_positive_decimal_for_savings(raw: &str, field: &str) -> Result<f64> {
    let normalized = normalize_positive_decimal_for_savings(raw, field)?;
    normalized.parse::<f64>().map_err(|_| {
        anyhow!("SAVINGS_PLAN_INPUT_INVALID: field '{field}' must be a positive decimal")
    })
}

fn normalize_non_negative_decimal_for_savings(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field '{field}' must be a non-negative decimal");
    }
    let dot_count = trimmed.chars().filter(|c| *c == '.').count();
    let has_only_decimal_chars = trimmed.chars().all(|c| c.is_ascii_digit() || c == '.');
    if !has_only_decimal_chars || dot_count > 1 {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field '{field}' must be a non-negative decimal");
    }
    let parsed = trimmed.parse::<f64>().map_err(|_| {
        anyhow!("SAVINGS_PLAN_INPUT_INVALID: field '{field}' must be a non-negative decimal")
    })?;
    if !parsed.is_finite() || parsed < 0.0 {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field '{field}' must be a non-negative decimal");
    }
    Ok(trimmed.to_string())
}

fn normalize_year_month_for_savings(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.len() != 7 || trimmed.as_bytes().get(4) != Some(&b'-') {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field 'year_month' must use YYYY-MM format");
    }
    let year = &trimmed[0..4];
    let month = &trimmed[5..7];
    let valid_year = year.chars().all(|c| c.is_ascii_digit());
    let valid_month = matches!(
        month,
        "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
    );
    if !valid_year || !valid_month {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field 'year_month' must use YYYY-MM format");
    }
    Ok(trimmed.to_string())
}

fn parse_broker_savings_plan_config(value: &Value) -> Result<ParsedBrokerSavingsPlanConfig> {
    let min_amount = value
        .get("minSavingsPlanAmount")
        .ok_or_else(|| {
            anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: missing minSavingsPlanAmount in config")
        })
        .and_then(parse_number_value_for_savings)?;
    let max_amount = value
        .get("maxSavingsPlanAmount")
        .ok_or_else(|| {
            anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: missing maxSavingsPlanAmount in config")
        })
        .and_then(parse_number_value_for_savings)?;
    if min_amount > max_amount {
        bail!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: invalid amount range in config");
    }

    let frequencies = parse_string_array_for_savings(
        value.get("frequencies"),
        "SAVINGS_PLAN_CONFIG_UNAVAILABLE: missing frequencies in config",
    )?;
    let payment_methods = parse_string_array_for_savings(
        value.get("paymentMethods"),
        "SAVINGS_PLAN_CONFIG_UNAVAILABLE: missing paymentMethods in config",
    )?;
    let dynamization_rates = parse_string_array_for_savings(
        value.get("dynamizationRates"),
        "SAVINGS_PLAN_CONFIG_UNAVAILABLE: missing dynamizationRates in config",
    )?;
    let default_dynamization_rate = value
        .get("defaultDynamizationRate")
        .ok_or_else(|| {
            anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: missing defaultDynamizationRate in config")
        })
        .and_then(parse_number_value_for_savings)?
        .to_string();

    let schedules = value
        .get("schedules")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: missing schedules in config"))?
        .iter()
        .map(parse_broker_savings_plan_schedule)
        .collect::<Result<Vec<_>>>()?;
    if schedules.is_empty() {
        bail!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: no schedules available in config");
    }

    Ok(ParsedBrokerSavingsPlanConfig {
        min_amount,
        max_amount,
        frequencies,
        payment_methods,
        dynamization_rates,
        default_dynamization_rate,
        schedules,
    })
}

fn parse_broker_savings_plan_schedule(value: &Value) -> Result<ParsedBrokerSavingsPlanSchedule> {
    let day_of_month = value
        .get("dayOfTheMonth")
        .and_then(Value::as_u64)
        .and_then(|v| u8::try_from(v).ok())
        .filter(|v| (1..=31).contains(v))
        .ok_or_else(|| {
            anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: invalid dayOfTheMonth in schedules")
        })?;

    let is_default = value
        .get("isDefault")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let is_earliest = value
        .get("isEarliest")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let available_year_months = value
        .get("yearMonths")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: missing yearMonths in schedule"))?
        .iter()
        .filter_map(|item| {
            let is_available = item
                .get("isAvailable")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !is_available {
                return None;
            }
            item.get("yearMonth")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();

    Ok(ParsedBrokerSavingsPlanSchedule {
        day_of_month,
        is_default,
        is_earliest,
        available_year_months,
    })
}

fn parse_number_value_for_savings(value: &Value) -> Result<f64> {
    let parsed = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(raw) => raw.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|v| v.is_finite() && *v >= 0.0)
    .ok_or_else(|| anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: invalid decimal value in config"))?;
    Ok(parsed)
}

fn parse_string_array_for_savings(raw: Option<&Value>, error: &str) -> Result<Vec<String>> {
    let values = raw
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("{error}"))?
        .iter()
        .filter_map(|value| match value {
            Value::String(raw) => Some(raw.trim().to_string()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty() {
        bail!("{error}");
    }
    Ok(values)
}

fn validate_amount_against_config(
    amount: f64,
    config: &ParsedBrokerSavingsPlanConfig,
) -> Result<()> {
    if amount < config.min_amount || amount > config.max_amount {
        bail!(
            "SAVINGS_PLAN_INPUT_INVALID: field 'amount' must be between {} and {}",
            config.min_amount,
            config.max_amount
        );
    }
    Ok(())
}

fn resolve_effective_savings_plan_add_config(
    args: &crate::cli::BrokerSavingsPlanAddArgs,
    config: &ParsedBrokerSavingsPlanConfig,
) -> Result<ResolvedSavingsPlanAddConfig> {
    let frequency = match args.frequency {
        Some(value) => {
            let gql = value.as_graphql().to_string();
            if !config.frequencies.iter().any(|f| f == &gql) {
                bail!("SAVINGS_PLAN_INPUT_INVALID: field 'frequency' is not allowed");
            }
            gql
        }
        None => {
            if config.frequencies.iter().any(|f| f == "MONTHLY") {
                "MONTHLY".to_string()
            } else {
                config.frequencies[0].clone()
            }
        }
    };

    let selected_schedule = match args.day_of_month {
        Some(day) => config
            .schedules
            .iter()
            .find(|schedule| schedule.day_of_month == day)
            .ok_or_else(|| {
                anyhow!("SAVINGS_PLAN_INPUT_INVALID: field 'day_of_month' is not allowed")
            })?,
        None => config
            .schedules
            .iter()
            .find(|schedule| schedule.is_default)
            .or_else(|| {
                config
                    .schedules
                    .iter()
                    .find(|schedule| schedule.is_earliest)
            })
            .or_else(|| {
                config
                    .schedules
                    .iter()
                    .min_by_key(|schedule| schedule.day_of_month)
            })
            .ok_or_else(|| {
                anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: no valid schedule found in config")
            })?,
    };

    let year_month = match args.year_month.as_deref() {
        Some(value) => {
            let normalized = normalize_year_month_for_savings(value)?;
            if !selected_schedule
                .available_year_months
                .iter()
                .any(|ym| ym == &normalized)
            {
                bail!(
                    "SAVINGS_PLAN_INPUT_INVALID: field 'year_month' is not available for selected day"
                );
            }
            normalized
        }
        None => selected_schedule
            .available_year_months
            .iter()
            .min()
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "SAVINGS_PLAN_CONFIG_UNAVAILABLE: no available yearMonth for selected schedule"
                )
            })?,
    };

    let dynamization_rate = match args.dynamization_rate.as_deref() {
        Some(value) => {
            let normalized =
                normalize_non_negative_decimal_for_savings(value, "dynamization_rate")?;
            if !config.dynamization_rates.is_empty()
                && !config
                    .dynamization_rates
                    .iter()
                    .any(|candidate| decimals_equal(candidate, &normalized))
            {
                bail!("SAVINGS_PLAN_INPUT_INVALID: field 'dynamization_rate' is not allowed");
            }
            normalized
        }
        None => normalize_non_negative_decimal_for_savings(
            config.default_dynamization_rate.as_str(),
            "dynamization_rate",
        )?,
    };

    let payment_method = match args.payment_method {
        Some(value) => {
            let gql = value.as_graphql().to_string();
            if !config.payment_methods.iter().any(|method| method == &gql) {
                bail!("SAVINGS_PLAN_INPUT_INVALID: field 'payment_method' is not allowed");
            }
            gql
        }
        None => {
            let preferred = "REFERENCE_ACCOUNT";
            if config
                .payment_methods
                .iter()
                .any(|method| method == preferred)
            {
                preferred.to_string()
            } else {
                config.payment_methods[0].clone()
            }
        }
    };

    Ok(ResolvedSavingsPlanAddConfig {
        frequency,
        day_of_month: selected_schedule.day_of_month,
        year_month,
        dynamization_rate,
        payment_method,
    })
}

fn decimals_equal(left: &str, right: &str) -> bool {
    let left_value = left.trim().parse::<f64>().ok();
    let right_value = right.trim().parse::<f64>().ok();
    match (left_value, right_value) {
        (Some(l), Some(r)) if l.is_finite() && r.is_finite() => (l - r).abs() <= 1e-12,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Session, StoredSession};
    use mockito::Server;
    use serde_json::json;

    fn sample_args() -> crate::cli::BrokerSavingsPlanAddArgs {
        crate::cli::BrokerSavingsPlanAddArgs {
            portfolio_id: None,
            isin: "US0378331005".to_string(),
            amount: "100".to_string(),
            frequency: None,
            day_of_month: None,
            year_month: None,
            dynamization_rate: None,
            payment_method: None,
            appropriateness_id: None,
            acknowledged_appropriateness_warning_version: None,
            json: false,
        }
    }

    fn sample_config() -> ParsedBrokerSavingsPlanConfig {
        ParsedBrokerSavingsPlanConfig {
            min_amount: 1.0,
            max_amount: 10_000.0,
            frequencies: vec!["MONTHLY".to_string(), "QUARTERLY".to_string()],
            payment_methods: vec![
                "BUYING_POWER_WITH_REFERENCE_ACCOUNT_FALLBACK".to_string(),
                "REFERENCE_ACCOUNT".to_string(),
            ],
            dynamization_rates: vec!["0".to_string(), "1.5".to_string()],
            default_dynamization_rate: "0".to_string(),
            schedules: vec![ParsedBrokerSavingsPlanSchedule {
                day_of_month: 5,
                is_default: true,
                is_earliest: true,
                available_year_months: vec!["2026-04".to_string(), "2026-05".to_string()],
            }],
        }
    }

    #[test]
    fn normalize_year_month_for_savings_rejects_invalid() {
        let err = normalize_year_month_for_savings("2026-13").unwrap_err();
        assert!(err.to_string().contains("YYYY-MM"));
    }

    #[test]
    fn normalize_year_month_for_savings_accepts_generated_values() {
        for year in [0_u16, 1, 2026, 9999] {
            for month in 1_u8..=12 {
                let value = format!("{year:04}-{month:02}");
                let normalized =
                    normalize_year_month_for_savings(&value).expect("valid year-month");
                assert_eq!(normalized, value);
            }
        }
    }

    #[test]
    fn normalize_year_month_for_savings_rejects_generated_invalid_months() {
        for year in [0_u16, 2026, 9999] {
            for month in [0_u8, 13, 42, 99] {
                let value = format!("{year:04}-{month:02}");
                let err = normalize_year_month_for_savings(&value)
                    .expect_err("invalid month should fail");
                assert!(err.to_string().contains("YYYY-MM"));
            }
        }
    }

    #[test]
    fn normalize_positive_decimal_for_savings_accepts_generated_positive_values() {
        for value in ["1", "10", "999999", "1.1", "42.125", "1000.0001"] {
            let normalized = normalize_positive_decimal_for_savings(value, "amount")
                .expect("generated positive decimal");
            assert_eq!(normalized, value);
        }
    }

    #[test]
    fn resolve_effective_savings_plan_add_config_uses_defaults() {
        let args = sample_args();
        let config = sample_config();
        let resolved = resolve_effective_savings_plan_add_config(&args, &config).expect("resolve");
        assert_eq!(resolved.frequency, "MONTHLY");
        assert_eq!(resolved.day_of_month, 5);
        assert_eq!(resolved.year_month, "2026-04");
        assert_eq!(resolved.payment_method, "REFERENCE_ACCOUNT");
    }

    #[test]
    fn resolve_effective_savings_plan_add_config_picks_earliest_available_year_month() {
        let args = sample_args();
        let mut config = sample_config();
        config.schedules[0].available_year_months = vec![
            "2026-06".to_string(),
            "2026-04".to_string(),
            "2026-05".to_string(),
        ];

        let resolved = resolve_effective_savings_plan_add_config(&args, &config).expect("resolve");

        assert_eq!(resolved.year_month, "2026-04");
    }

    #[test]
    fn validate_amount_against_config_rejects_outside_range() {
        let config = sample_config();
        let err = validate_amount_against_config(0.5, &config).unwrap_err();
        assert!(err.to_string().contains("between"));
    }

    #[test]
    fn parse_broker_savings_plan_config_accepts_numeric_default_dynamization_rate() {
        let config = json!({
            "minSavingsPlanAmount": 1,
            "maxSavingsPlanAmount": 5000,
            "frequencies": ["MONTHLY"],
            "paymentMethods": ["REFERENCE_ACCOUNT"],
            "dynamizationRates": [0, 2, 3, 5],
            "defaultDynamizationRate": 0,
            "schedules": [{
                "dayOfTheMonth": 1,
                "isDefault": true,
                "isEarliest": true,
                "yearMonths": [{
                    "yearMonth": "2026-04",
                    "isAvailable": true
                }]
            }]
        });

        let parsed = parse_broker_savings_plan_config(&config).expect("parse config");

        assert_eq!(parsed.default_dynamization_rate, "0");
        assert_eq!(parsed.dynamization_rates, vec!["0", "2", "3", "5"]);
    }

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: String) -> Self {
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => unsafe {
                    std::env::set_var(self.key, value);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    struct TestChannelGuard {
        _override: crate::channel::TestEnvConfigOverrideGuard,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestChannelGuard {
        fn for_server(server: &Server) -> Self {
            let lock = crate::lock_test_env();
            let env_cfg = crate::config::EnvConfig {
                graphql_url: server.url(),
                auth: crate::config::AuthConfig {
                    issuer: "https://issuer.test".to_string(),
                    audience: "https://audience.test".to_string(),
                    client_id: "client-id".to_string(),
                },
            };
            Self {
                _lock: lock,
                _override: crate::channel::TestEnvConfigOverrideGuard::set(env_cfg),
            }
        }
    }

    fn sample_session() -> Session {
        Session {
            access_token: "test-token".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: crate::session::LoginSource::DeviceCode,
        }
    }

    fn sample_env_cfg(graphql_url: String) -> EnvConfig {
        EnvConfig {
            graphql_url,
            auth: crate::config::AuthConfig {
                issuer: "https://issuer.test".to_string(),
                audience: "https://audience.test".to_string(),
                client_id: "client-id".to_string(),
            },
        }
    }

    fn sample_runtime_config() -> AppConfig {
        AppConfig {
            auth: crate::config::RuntimeAuthConfig {
                session_backend: crate::config::SessionBackendPreference::File,
                signing_key_backend: crate::config::DpopKeyBackend::File,
                pkcs11: None,
            },
        }
    }

    fn file_session_manager(tempdir: &tempfile::TempDir) -> SessionManager {
        SessionManager::with_store(crate::session::StorageBackend::File(
            crate::session::FileStore::new(tempdir.path().to_path_buf()).expect("file store"),
        ))
    }

    fn expected_authorization_header() -> &'static str {
        "DPoP test-token"
    }

    fn ensure_runtime_dpop_key(config: &AppConfig) {
        crate::dpop::DpopKeyMaterial::load_or_create_for_options(
            &crate::channel::current_dpop_runtime_options(config),
        )
        .expect("create runtime dpop key");
    }

    fn current_runtime_dpop_thumbprint(config: &AppConfig) -> String {
        crate::dpop::DpopKeyMaterial::load_existing_for_options(
            &crate::channel::current_dpop_runtime_options(config),
        )
        .expect("load runtime dpop key")
        .jwk_thumbprint()
        .expect("runtime dpop thumbprint")
    }

    #[test]
    fn resolve_broker_ids_auto_resolves_single_portfolio() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let mut server = Server::new();

        let resolve_mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "account": {
                            "id": "acc-1",
                            "brokerPortfolios": [{ "id": "p-1" }]
                        }
                    }
                }"#,
            )
            .create();

        let config = sample_runtime_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        let mut session = sample_session();
        let env_cfg = sample_env_cfg(server.url());

        let resolved = resolve_broker_ids(
            &mut session_manager,
            TargetEnv::Dev,
            &env_cfg,
            &mut session,
            &crate::channel::current_dpop_runtime_options(&config),
            None,
        )
        .expect("resolve broker ids");

        assert_eq!(resolved.account_id, "acc-1");
        assert_eq!(resolved.portfolio_id, "p-1");
        assert_eq!(resolved.portfolio_source, "auto_resolve");
        resolve_mock.assert();
    }

    #[test]
    fn resolve_broker_ids_errors_when_multiple_portfolios_resolved_without_selection() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let mut server = Server::new();

        let resolve_mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "account": {
                            "id": "acc-1",
                            "brokerPortfolios": [{ "id": "p-2" }, { "id": "p-1" }]
                        }
                    }
                }"#,
            )
            .create();

        let config = sample_runtime_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        let mut session = sample_session();
        let env_cfg = sample_env_cfg(server.url());

        let err = match resolve_broker_ids(
            &mut session_manager,
            TargetEnv::Dev,
            &env_cfg,
            &mut session,
            &crate::channel::current_dpop_runtime_options(&config),
            None,
        ) {
            Ok(_) => panic!("multiple portfolios should fail closed"),
            Err(err) => err,
        };

        let message = err.to_string();
        assert!(message.contains("Unable to resolve broker portfolio id"));
        assert!(message.contains("multiple portfolios found [p-1, p-2]"));
        assert!(message.contains("Provide --portfolio-id"));
        resolve_mock.assert();
    }

    #[test]
    fn resolve_price_alert_lookup_from_items_resolves_security_match() {
        let security_items = vec![json!({
            "alert_id": "alert-1",
            "isin": "US0378331005"
        })];
        let crypto_items = vec![];

        let resolved =
            resolve_price_alert_lookup_from_items("alert-1", &security_items, &crypto_items)
                .expect("resolved");

        assert_eq!(
            resolved,
            ResolvedPriceAlert::Security {
                alert_id: "alert-1".to_string(),
                isin: "US0378331005".to_string(),
            }
        );
    }

    #[test]
    fn resolve_price_alert_lookup_from_items_routes_crypto_match() {
        let security_items = vec![];
        let crypto_items = vec![json!({
            "alert_id": "alert-1",
            "ticker": "BTC"
        })];

        let resolved =
            resolve_price_alert_lookup_from_items("alert-1", &security_items, &crypto_items)
                .expect("resolved");

        assert_eq!(
            resolved,
            ResolvedPriceAlert::Crypto {
                alert_id: "alert-1".to_string(),
                ticker: "BTC".to_string(),
            }
        );
    }

    #[test]
    fn resolve_price_alert_lookup_from_items_rejects_unknown_id() {
        let err = resolve_price_alert_lookup_from_items("missing", &[], &[]).unwrap_err();
        assert!(
            err.to_string()
                .contains("was not found in the active portfolio")
        );
    }

    #[test]
    fn resolve_price_alert_lookup_from_items_rejects_duplicate_cross_kind_matches() {
        let security_items = vec![json!({
            "alert_id": "alert-1",
            "isin": "US0378331005"
        })];
        let crypto_items = vec![json!({
            "alert_id": "alert-1",
            "ticker": "BTC"
        })];

        let err = resolve_price_alert_lookup_from_items("alert-1", &security_items, &crypto_items)
            .unwrap_err();
        assert!(err.to_string().contains("matched multiple alert kinds"));
    }

    #[test]
    fn resolve_price_alert_lookup_with_crypto_loader_short_circuits_after_security_match() {
        let security_items = vec![json!({
            "alert_id": "alert-1",
            "isin": "US0378331005"
        })];

        let resolved = resolve_price_alert_lookup_with_crypto_loader(
            "alert-1",
            &security_items,
            || -> Result<Vec<Value>> {
                panic!("crypto lookup should not run when security already matched");
            },
        )
        .expect("resolved");

        assert_eq!(
            resolved,
            ResolvedPriceAlert::Security {
                alert_id: "alert-1".to_string(),
                isin: "US0378331005".to_string(),
            }
        );
    }

    #[test]
    fn resolve_price_alert_lookup_with_crypto_loader_uses_crypto_when_security_missing() {
        let security_items = vec![];

        let resolved =
            resolve_price_alert_lookup_with_crypto_loader("alert-1", &security_items, || {
                Ok(vec![json!({
                    "alert_id": "alert-1",
                    "ticker": "BTC"
                })])
            })
            .expect("resolved");

        assert_eq!(
            resolved,
            ResolvedPriceAlert::Crypto {
                alert_id: "alert-1".to_string(),
                ticker: "BTC".to_string(),
            }
        );
    }

    #[test]
    fn run_broker_command_human_routes_trade_cancel_to_text_output() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());

        let cancel_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_authorization_header())
            .match_body(mockito::Matcher::Regex("BrokerCancelOrder".to_string()))
            .match_body(mockito::Matcher::PartialJson(json!({
                "variables": {
                    "portfolioId": "portfolio-1",
                    "orderId": "order-1"
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "cancelOrder": {
                            "id": "portfolio-1"
                        }
                    }
                }"#,
            )
            .create();

        let config = sample_runtime_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        session_manager
            .save_active(&StoredSession {
                env: crate::channel::current_env(),
                session: sample_session(),
                dpop_jwk_thumbprint: Some(current_runtime_dpop_thumbprint(&config)),
                mode: None,
            })
            .expect("save session");

        let output = run_broker_command_human(
            crate::cli::BrokerArgs {
                command: crate::cli::BrokerCommand::Trade(crate::cli::BrokerTradeArgs {
                    command: crate::cli::BrokerTradeCommand::Cancel(
                        crate::cli::BrokerTradeCancelArgs {
                            order_id: "order-1".to_string(),
                            portfolio_id: Some("portfolio-1".to_string()),
                            json: false,
                        },
                    ),
                }),
            },
            &config,
            &mut session_manager,
        )
        .expect("human cancel output");

        match output {
            HumanBrokerOutput::Text(lines) => {
                assert_eq!(lines, vec!["Cancellation requested.", "order_id: order-1"]);
            }
            HumanBrokerOutput::Json(_, _) => panic!("expected text output for human cancel path"),
        }

        cancel_mock.assert();
    }

    #[test]
    fn render_broker_transaction_details_text_includes_nested_fee_and_total_tax_fields() {
        let payload = json!({
            "result": {
                "id": "tx-42",
                "transaction_reference": "WUM 872598752",
                "type": "TRADE",
                "detail_type": "security_trade",
                "currency": "EUR",
                "last_event_datetime": "2026-04-15T10:22:31Z",
                "security": {
                    "isin": "IE00B4ND3602",
                    "name": "Example ETF",
                    "security_type": "ETF"
                },
                "security_trade": {
                    "status": "FILLED",
                    "side": "BUY",
                    "order_kind": "SINGLE",
                    "number_of_shares": {
                        "filled": "10",
                        "total": "10"
                    },
                    "average_price": "10.25",
                    "total_amount": "102.50",
                    "finalisation_reason": Value::Null,
                    "limit_price": Value::Null,
                    "stop_price": Value::Null,
                    "valid_until": Value::Null,
                    "is_cancellation_requested": false,
                    "trading_venue": "MUNC",
                    "fee": "1.11",
                    "transactional_fee": "0.22",
                    "taxes": "2.50",
                    "trade_transaction_amounts": {
                        "tax_amount": "2.50",
                        "transaction_fee": "0.99",
                        "venue_fee": "1.01",
                        "crypto_spread_fee": "0.03"
                    },
                    "aggregated_transaction_taxes": {
                        "total_tax": "2.50",
                        "capital_gains_tax": "2.10",
                        "church_tax": "0.10",
                        "solidarity_tax": "0.30",
                        "source_tax": Value::Null,
                        "financial_transaction_tax": Value::Null
                    }
                },
                "documents": [],
                "linked_transaction_ids": [],
                "history": []
            }
        });

        let lines = render_broker_transaction_details_text(&payload);

        assert!(lines.iter().any(|line| line == "transaction_fee: 0.99 EUR"));
        assert!(lines.iter().any(|line| line == "venue_fee: 1.01 EUR"));
        assert!(
            lines
                .iter()
                .any(|line| line == "crypto_spread_fee: 0.03 EUR")
        );
        assert!(lines.iter().any(|line| line == "total_tax: 2.50 EUR"));
    }

    #[test]
    fn render_broker_transaction_details_text_shows_missing_nested_fee_and_total_tax_fields_as_none()
     {
        let payload = json!({
            "result": {
                "id": "tx-42",
                "transaction_reference": "WUM 872598752",
                "type": "TRADE",
                "detail_type": "security_trade",
                "currency": "EUR",
                "last_event_datetime": "2026-04-15T10:22:31Z",
                "security": {
                    "isin": "IE00B4ND3602",
                    "name": "Example ETF",
                    "security_type": "ETF"
                },
                "security_trade": {
                    "status": "FILLED",
                    "side": "BUY",
                    "order_kind": "SINGLE",
                    "number_of_shares": {
                        "filled": "10",
                        "total": "10"
                    },
                    "average_price": "10.25",
                    "total_amount": "102.50",
                    "finalisation_reason": Value::Null,
                    "limit_price": Value::Null,
                    "stop_price": Value::Null,
                    "valid_until": Value::Null,
                    "is_cancellation_requested": false,
                    "trading_venue": "MUNC",
                    "fee": Value::Null,
                    "transactional_fee": Value::Null,
                    "taxes": Value::Null,
                    "trade_transaction_amounts": {
                        "tax_amount": Value::Null,
                        "transaction_fee": Value::Null,
                        "venue_fee": Value::Null,
                        "crypto_spread_fee": Value::Null
                    },
                    "aggregated_transaction_taxes": {
                        "total_tax": Value::Null,
                        "capital_gains_tax": Value::Null,
                        "church_tax": Value::Null,
                        "solidarity_tax": Value::Null,
                        "source_tax": Value::Null,
                        "financial_transaction_tax": Value::Null
                    }
                },
                "documents": [],
                "linked_transaction_ids": [],
                "history": []
            }
        });

        let lines = render_broker_transaction_details_text(&payload);

        assert!(lines.iter().any(|line| line == "transaction_fee: <none>"));
        assert!(lines.iter().any(|line| line == "venue_fee: <none>"));
        assert!(lines.iter().any(|line| line == "crypto_spread_fee: <none>"));
        assert!(lines.iter().any(|line| line == "total_tax: <none>"));
    }
}
