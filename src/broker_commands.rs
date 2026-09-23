use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::active_session::load_active_session;
use crate::broker_context::{
    BrokerContext, context_file_path, load_context as load_broker_context,
    save_context as save_broker_context,
};
use crate::broker_portfolio_groups::{
    execute_broker_portfolio_groups, execute_broker_portfolio_groups_assign,
    execute_broker_portfolio_groups_create, execute_broker_portfolio_groups_delete,
    execute_broker_portfolio_groups_unassign, execute_broker_portfolio_groups_update,
};
use crate::broker_portfolio_groups_render::{
    render_broker_portfolio_groups_mutation_text, render_broker_portfolio_groups_text,
};
use crate::broker_query_execution::{
    execute_broker_cash_breakdown as execute_broker_cash_breakdown_query,
    execute_broker_chart as execute_broker_chart_query,
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
    RESOLVE_BROKER_IDS_QUERY, ResolvedBrokerIds, resolve_broker_ids, validated_broker_input,
};
use crate::cli::{
    BrokerArgs, BrokerCommand, BrokerContextCommand, BrokerDerivativesCommand,
    BrokerPortfolioGroupsCommand, BrokerPriceAlertsCommand, BrokerSavingsPlansCommand,
    BrokerTradeCommand, BrokerTransactionCommand, BrokerWatchlistCommand,
};
use crate::config::{AppConfig, EnvConfig, TargetEnv};
use crate::graphql::{
    enforce_graphql_access_policy, execute_graphql, execute_graphql_once_with_headers,
};
use crate::helpers::{
    BROKER_ADD_CRYPTO_PRICE_ALERT_MUTATION, BROKER_ADD_PRICE_ALERT_MUTATION,
    BROKER_ADD_TO_WATCHLIST_MUTATION, BROKER_CREATE_OR_UPDATE_SAVINGS_PLAN_MUTATION,
    BROKER_CRYPTO_PRICE_ALERTS_QUERY, BROKER_PRICE_ALERTS_QUERY,
    BROKER_REMOVE_CRYPTO_PRICE_ALERT_MUTATION, BROKER_REMOVE_FROM_WATCHLIST_MUTATION,
    BROKER_REMOVE_PRICE_ALERT_MUTATION, BROKER_REMOVE_SAVINGS_PLAN_MUTATION,
    BROKER_SAVINGS_PLAN_BY_ISIN_QUERY, BROKER_SAVINGS_PLAN_CONFIG_QUERY,
    BROKER_SAVINGS_PLAN_EX_ANTE_COSTS_QUERY, SAVINGS_PLAN_EXECUTION_VENUE,
    broker_add_crypto_price_alert_variables, broker_add_price_alert_variables,
    broker_add_to_watchlist_variables, broker_create_or_update_savings_plan_variables,
    broker_crypto_price_alerts_variables, broker_remove_from_watchlist_variables,
    broker_remove_price_alert_variables, broker_remove_savings_plan_variables,
    broker_savings_plan_by_isin_variables, broker_savings_plan_config_variables,
    broker_savings_plan_ex_ante_cost_variables, project_broker_add_crypto_price_alert_response,
    project_broker_add_price_alert_response, project_broker_create_or_update_savings_plan_response,
    project_broker_crypto_price_alerts_response, project_broker_remove_crypto_price_alert_response,
    project_broker_remove_price_alert_response, project_broker_remove_savings_plan_response,
    project_broker_savings_plan_by_isin_response,
    project_broker_savings_plan_config_details_response,
    project_broker_savings_plan_ex_ante_costs_response, project_broker_watchlist_add_response,
    project_broker_watchlist_remove_response,
};
use crate::payload_fingerprint::checksum_for_payload;
use crate::resolve_active_env;
use crate::savings_plan_confirmation::{
    SavingsPlanConfirmation, assert_preview_allowed, finalize_consumed, finalize_unknown,
    load_pending as load_savings_plan_confirmation, start_submission, store_pending,
};
use crate::savings_plan_presentation::{
    COMPLIANCE_RULE_ID as SAVINGS_PLAN_COMPLIANCE_RULE_ID,
    PRESENTATION_FORMAT as SAVINGS_PLAN_PRESENTATION_FORMAT,
    build_phase1_presentation as build_savings_plan_presentation,
    render_phase1_text as render_savings_plan_preview_text,
    required_leaf_paths as savings_plan_required_leaf_paths,
};
use crate::session::{Session, SessionManager};
use crate::session_refresh::execute_with_refresh_retry;
use crate::trade_execution::{
    execute_broker_trade_buy, execute_broker_trade_cancel, execute_broker_trade_sell,
    render_trade_buy_text, render_trade_cancel_text, render_trade_sell_text,
};

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
        BrokerCommand::CashBreakdown(cash_breakdown_args) => {
            let compact = cash_breakdown_args.json;
            let payload =
                execute_broker_cash_breakdown(cash_breakdown_args, config, session_manager)?;
            if compact {
                Ok(HumanBrokerOutput::Json(payload, true))
            } else {
                Ok(HumanBrokerOutput::Text(render_broker_cash_breakdown_text(
                    &payload,
                )))
            }
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
        BrokerCommand::PortfolioGroups(portfolio_groups_args) => {
            let crate::cli::BrokerPortfolioGroupsArgs {
                command,
                portfolio_id,
                group_id,
                json,
            } = portfolio_groups_args;
            match command {
                Some(BrokerPortfolioGroupsCommand::Create(create_args)) => {
                    let payload = execute_broker_portfolio_groups_create(
                        create_args,
                        config,
                        session_manager,
                    )?;
                    Ok(HumanBrokerOutput::Text(
                        render_broker_portfolio_groups_mutation_text(&payload),
                    ))
                }
                Some(BrokerPortfolioGroupsCommand::Update(update_args)) => {
                    let payload = execute_broker_portfolio_groups_update(
                        update_args,
                        config,
                        session_manager,
                    )?;
                    Ok(HumanBrokerOutput::Text(
                        render_broker_portfolio_groups_mutation_text(&payload),
                    ))
                }
                Some(BrokerPortfolioGroupsCommand::Delete(delete_args)) => {
                    let payload = execute_broker_portfolio_groups_delete(
                        delete_args,
                        config,
                        session_manager,
                    )?;
                    Ok(HumanBrokerOutput::Text(
                        render_broker_portfolio_groups_mutation_text(&payload),
                    ))
                }
                Some(BrokerPortfolioGroupsCommand::Assign(assign_args)) => {
                    let payload = execute_broker_portfolio_groups_assign(
                        assign_args,
                        config,
                        session_manager,
                    )?;
                    Ok(HumanBrokerOutput::Text(
                        render_broker_portfolio_groups_mutation_text(&payload),
                    ))
                }
                Some(BrokerPortfolioGroupsCommand::Unassign(unassign_args)) => {
                    let payload = execute_broker_portfolio_groups_unassign(
                        unassign_args,
                        config,
                        session_manager,
                    )?;
                    Ok(HumanBrokerOutput::Text(
                        render_broker_portfolio_groups_mutation_text(&payload),
                    ))
                }
                None => {
                    let filtered_to_group = group_id
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty());
                    let payload = execute_broker_portfolio_groups(
                        crate::cli::BrokerPortfolioGroupsArgs {
                            command: None,
                            portfolio_id,
                            group_id,
                            json,
                        },
                        config,
                        session_manager,
                    )?;
                    Ok(HumanBrokerOutput::Text(
                        render_broker_portfolio_groups_text(&payload, filtered_to_group),
                    ))
                }
            }
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
        BrokerCommand::Chart(chart_args) => {
            let compact = chart_args.json;
            let payload = execute_broker_chart(chart_args, config, session_manager)?;
            if compact {
                Ok(HumanBrokerOutput::Json(payload, true))
            } else {
                Ok(HumanBrokerOutput::Text(render_broker_chart_text(&payload)))
            }
        }
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
                    let is_preview = add_args.confirm.is_none();
                    let payload =
                        execute_broker_savings_plan_add(add_args, config, session_manager)?;
                    if is_preview {
                        Ok(HumanBrokerOutput::Text(render_savings_plan_preview_text(
                            &payload,
                        )))
                    } else {
                        Ok(HumanBrokerOutput::Json(payload, false))
                    }
                }
                Some(BrokerSavingsPlansCommand::Config(config_args)) => {
                    let compact = config_args.json;
                    let payload =
                        execute_broker_savings_plan_config(config_args, config, session_manager)?;
                    if compact {
                        Ok(HumanBrokerOutput::Json(payload, true))
                    } else {
                        Ok(HumanBrokerOutput::Text(
                            render_broker_savings_plan_config_text(&payload),
                        ))
                    }
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
        BrokerCommand::CashBreakdown(args) => {
            execute_broker_cash_breakdown(args, config, session_manager)
        }
        BrokerCommand::Transactions(args) => {
            execute_broker_transactions(args, config, session_manager)
        }
        BrokerCommand::Transaction(transaction_args) => match transaction_args.command {
            BrokerTransactionCommand::Details(args) => {
                execute_broker_transaction_details(args, config, session_manager)
            }
        },
        BrokerCommand::Holdings(args) => execute_broker_holdings(args, config, session_manager),
        BrokerCommand::PortfolioGroups(args) => {
            let crate::cli::BrokerPortfolioGroupsArgs {
                command,
                portfolio_id,
                group_id,
                json,
            } = args;
            match command {
                Some(BrokerPortfolioGroupsCommand::Create(args)) => {
                    execute_broker_portfolio_groups_create(args, config, session_manager)
                }
                Some(BrokerPortfolioGroupsCommand::Update(args)) => {
                    execute_broker_portfolio_groups_update(args, config, session_manager)
                }
                Some(BrokerPortfolioGroupsCommand::Delete(args)) => {
                    execute_broker_portfolio_groups_delete(args, config, session_manager)
                }
                Some(BrokerPortfolioGroupsCommand::Assign(args)) => {
                    execute_broker_portfolio_groups_assign(args, config, session_manager)
                }
                Some(BrokerPortfolioGroupsCommand::Unassign(args)) => {
                    execute_broker_portfolio_groups_unassign(args, config, session_manager)
                }
                None => execute_broker_portfolio_groups(
                    crate::cli::BrokerPortfolioGroupsArgs {
                        command: None,
                        portfolio_id,
                        group_id,
                        json,
                    },
                    config,
                    session_manager,
                ),
            }
        }
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
        BrokerCommand::Chart(args) => execute_broker_chart(args, config, session_manager),
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
                Some(BrokerSavingsPlansCommand::Config(config_args)) => {
                    execute_broker_savings_plan_config(config_args, config, session_manager)
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

fn render_broker_cash_breakdown_text(payload: &Value) -> Vec<String> {
    let result = payload.get("result").unwrap_or(payload);

    vec![
        format!(
            "cash_balance: {}",
            display_value(result.get("cash_balance"))
        ),
        format!(
            "buying_power: {}",
            display_value(result.get("buying_power"))
        ),
        format!(
            "buying_power_without_credit: {}",
            display_value(result.get("buying_power_without_credit"))
        ),
        format!(
            "available_credit_line: {}",
            display_value(result.get("available_credit_line"))
        ),
        format!("loaned: {}", display_value(result.get("loaned"))),
        format!(
            "pending_buy_orders_amount: {}",
            display_value(result.get("pending_buy_orders_amount"))
        ),
        format!(
            "possible_taxes: {}",
            display_value(result.get("possible_taxes"))
        ),
        format!(
            "derivatives_buying_power: {}",
            display_value(result.get("derivatives_buying_power"))
        ),
        format!(
            "available_for_derivatives: {}",
            display_value(result.get("available_for_derivatives"))
        ),
    ]
}

fn render_broker_savings_plan_config_text(payload: &Value) -> Vec<String> {
    let result = payload.get("result").unwrap_or(payload);
    let security = result.get("security").unwrap_or(&Value::Null);
    let amount_limits = result.get("amount_limits").unwrap_or(&Value::Null);
    let defaults = result.get("defaults").unwrap_or(&Value::Null);
    let schedules = result
        .get("schedules")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let mut lines = vec![
        format!(
            "portfolio_id: {}",
            display_value(payload.get("portfolio_id"))
        ),
        format!("security_isin: {}", display_value(security.get("isin"))),
        format!("security_name: {}", display_value(security.get("name"))),
        format!(
            "security_type: {}",
            display_value(security.get("security_type"))
        ),
        format!("amount_min: {}", display_value(amount_limits.get("min"))),
        format!("amount_max: {}", display_value(amount_limits.get("max"))),
        format!(
            "default_frequency: {}",
            display_value(defaults.get("frequency"))
        ),
        format!(
            "default_day_of_month: {}",
            display_value(defaults.get("day_of_month"))
        ),
        format!(
            "default_year_month: {}",
            display_value(defaults.get("year_month"))
        ),
        format!(
            "default_dynamization_rate: {}",
            display_value(defaults.get("dynamization_rate"))
        ),
        format!(
            "default_payment_method: {}",
            display_value(defaults.get("payment_method"))
        ),
        format!(
            "frequencies: {}",
            display_string_list(result.get("frequencies"))
        ),
        format!(
            "payment_methods: {}",
            display_string_list(result.get("payment_methods"))
        ),
        format!(
            "dynamization_rates: {}",
            display_string_list(result.get("dynamization_rates"))
        ),
    ];

    for schedule in schedules {
        let day_of_month = schedule
            .get("day_of_month")
            .map(|value| display_value(Some(value)))
            .unwrap_or_else(|| "<none>".to_string());
        lines.push(format!(
            "schedule_{day_of_month}: day_of_month={} default={} earliest={} available_year_months={}",
            display_value(schedule.get("day_of_month")),
            display_value(schedule.get("is_default")),
            display_value(schedule.get("is_earliest")),
            display_string_list(schedule.get("available_year_months")),
        ));
    }

    lines
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

fn display_string_list(value: Option<&Value>) -> String {
    let values = value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if values.is_empty() {
        "<none>".to_string()
    } else {
        values.join(", ")
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

fn render_broker_chart_text(payload: &Value) -> Vec<String> {
    let result = payload.get("result").unwrap_or(payload);
    let data_points = result
        .get("data_points")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let currency = result.get("currency").and_then(Value::as_str);
    let mut timestamped_points = data_points
        .iter()
        .filter(|point| chart_point_timestamp_key(point).is_some())
        .collect::<Vec<_>>();
    timestamped_points.sort_by_key(|point| chart_point_timestamp_key(point));
    let first_point = timestamped_points.first().copied();
    let last_point = timestamped_points.last().copied();
    let min_point = data_points
        .iter()
        .filter_map(|point| chart_point_numeric_key(point).map(|value| (value, point)))
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, point)| point);
    let max_point = data_points
        .iter()
        .filter_map(|point| chart_point_numeric_key(point).map(|value| (value, point)))
        .max_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, point)| point);

    vec![
        format!("isin: {}", display_value(result.get("isin"))),
        format!("timeframe: {}", display_value(result.get("timeframe"))),
        format!("currency: {}", display_value(result.get("currency"))),
        format!("source: {}", display_value(result.get("source"))),
        format!("point_count: {}", display_value(result.get("point_count"))),
        format!(
            "range_start: {}",
            display_value(first_point.and_then(|point| point.get("timestamp_utc")))
        ),
        format!(
            "range_end: {}",
            display_value(last_point.and_then(|point| point.get("timestamp_utc")))
        ),
        format!(
            "first_mid_price: {}",
            display_money(
                first_point.and_then(|point| point.get("mid_price")),
                currency
            )
        ),
        format!(
            "last_mid_price: {}",
            display_money(
                last_point.and_then(|point| point.get("mid_price")),
                currency
            )
        ),
        format!(
            "min_mid_price: {}",
            display_money(min_point.and_then(|point| point.get("mid_price")), currency)
        ),
        format!(
            "max_mid_price: {}",
            display_money(max_point.and_then(|point| point.get("mid_price")), currency)
        ),
        "hint: rerun with --json to get the full point set".to_string(),
    ]
}

fn chart_point_numeric_key(point: &Value) -> Option<f64> {
    match point.get("mid_price")? {
        Value::Number(number) => number.as_f64(),
        Value::String(raw) => raw.parse::<f64>().ok(),
        _ => None,
    }
}

fn chart_point_timestamp_key(point: &Value) -> Option<&str> {
    point
        .get("timestamp_utc")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
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

pub(crate) fn execute_broker_chart(
    args: crate::cli::BrokerChartArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_chart_query(args, config, session_manager)
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

pub(crate) fn execute_broker_cash_breakdown(
    args: crate::cli::BrokerCashBreakdownArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_cash_breakdown_query(args, config, session_manager)
}

pub(crate) fn execute_broker_savings_plans(
    args: crate::cli::BrokerSavingsPlansArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_broker_savings_plans_query(args, config, session_manager)
}

struct LoadedBrokerSavingsPlanConfig {
    ids: ResolvedBrokerIds,
    requested_isin: String,
    response: Value,
}

fn broker_result_envelope(ids: &ResolvedBrokerIds, result: Value) -> Value {
    json!({
        "account_id": ids.account_id,
        "portfolio_id": ids.portfolio_id,
        "resolution": {
            "account": ids.account_source,
            "portfolio": ids.portfolio_source,
        },
        "result": result,
    })
}

fn load_broker_savings_plan_config_response(
    config: &AppConfig,
    session_manager: &mut SessionManager,
    portfolio_id: Option<&str>,
    isin: &str,
) -> Result<LoadedBrokerSavingsPlanConfig> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let mut session = loaded.session;
    let access_context = loaded.access_context;

    load_broker_savings_plan_config_response_with_session(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        access_context,
        dpop_options,
        portfolio_id,
        isin,
    )
}

#[allow(clippy::too_many_arguments)]
fn load_broker_savings_plan_config_response_with_session(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    session: &mut Session,
    access_context: crate::graphql::GraphqlAccessContext,
    dpop_options: &crate::dpop::DpopRuntimeOptions,
    portfolio_id: Option<&str>,
    isin: &str,
) -> Result<LoadedBrokerSavingsPlanConfig> {
    let requested_isin = isin.trim().to_string();
    if requested_isin.is_empty() {
        bail!("SAVINGS_PLAN_INPUT_INVALID: field 'isin' must be a non-empty string");
    }

    let ids = resolve_broker_ids(
        session_manager,
        env,
        env_cfg,
        session,
        dpop_options,
        portfolio_id,
    )?;
    let input = validated_broker_input(&ids, false, None)?;

    let config_variables = broker_savings_plan_config_variables(&input, &requested_isin)
        .map_err(|err| anyhow!("SAVINGS_PLAN_INPUT_INVALID: {err}"))?;
    let requested_isin = config_variables
        .get("isin")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("Broker response invalid: missing normalized savings-plan isin"))?;
    let response = execute_with_refresh_retry(
        session_manager,
        env,
        env_cfg,
        session,
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

    Ok(LoadedBrokerSavingsPlanConfig {
        ids,
        requested_isin,
        response,
    })
}

pub(crate) fn execute_broker_savings_plan_config(
    args: crate::cli::BrokerSavingsPlanConfigArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let loaded = load_broker_savings_plan_config_response(
        config,
        session_manager,
        args.portfolio_id.as_deref(),
        args.isin.as_str(),
    )?;

    let projected = project_broker_savings_plan_config_details_response(
        &loaded.requested_isin,
        &loaded.response,
    )
    .map_err(|_| {
            anyhow!(
                "SAVINGS_PLAN_CONFIG_UNAVAILABLE: savings plan is not available for this instrument in the selected portfolio context"
            )
        })?;
    let raw_config = projected
        .get("savings_plan_configuration")
        .cloned()
        .unwrap_or(Value::Null);
    let parsed_config = parse_broker_savings_plan_config(&raw_config).map_err(|_| {
        anyhow!(
            "SAVINGS_PLAN_CONFIG_UNAVAILABLE: savings plan is not available for this instrument in the selected portfolio context"
        )
    })?;
    let defaults = resolve_default_savings_plan_config(&parsed_config)?;
    let result = project_public_broker_savings_plan_config_response(
        &loaded.requested_isin,
        &projected,
        &parsed_config,
        &defaults,
    );

    Ok(broker_result_envelope(&loaded.ids, result))
}

pub(crate) fn execute_broker_savings_plan_add(
    args: crate::cli::BrokerSavingsPlanAddArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    if args.confirm.is_some() {
        execute_broker_savings_plan_add_phase2(args, config, session_manager)
    } else {
        execute_broker_savings_plan_add_phase1(args, config, session_manager)
    }
}

struct PreparedSavingsPlanAdd {
    ids: ResolvedBrokerIds,
    isin: String,
    amount: String,
    effective_configuration: Value,
    security: Value,
    costs: Value,
    snapshot: Value,
    snapshot_checksum: String,
}

fn execute_broker_savings_plan_add_phase1(
    args: crate::cli::BrokerSavingsPlanAddArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    assert_preview_allowed()?;
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded_session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let mut session = loaded_session.session;
    let access_context = loaded_session.access_context;
    let prepared = prepare_savings_plan_add(
        &args,
        session_manager,
        env,
        &env_cfg,
        &mut session,
        access_context,
        dpop_options,
    )?;
    let now_epoch = current_epoch_seconds();
    let confirmation_id = format!("scsp1_{:032x}", rand::random::<u128>());
    let expires_at_epoch = now_epoch + 15 * 60;
    let confirmation = SavingsPlanConfirmation {
        confirmation_id: confirmation_id.clone(),
        snapshot_checksum: prepared.snapshot_checksum.clone(),
        created_at_epoch: now_epoch,
        expires_at_epoch,
        env: env.as_str().to_string(),
        account_id: prepared.ids.account_id.clone(),
        portfolio_id: prepared.ids.portfolio_id.clone(),
        isin: prepared.isin.clone(),
        snapshot: prepared.snapshot.clone(),
    };
    store_pending(confirmation)?;

    let confirmation_payload = json!({
        "id": confirmation_id,
        "intent_checksum": prepared.snapshot_checksum,
        "expires_at_epoch": expires_at_epoch,
        "command_template": savings_plan_command_template(&args, confirmation_id.as_str(), args.json),
        "phase_2_command_template_json": savings_plan_command_template(&args, confirmation_id.as_str(), true),
    });
    let presentation = build_savings_plan_presentation(
        &prepared.security,
        prepared.amount.as_str(),
        &prepared.effective_configuration,
        SAVINGS_PLAN_EXECUTION_VENUE,
        &prepared.costs,
        &confirmation_payload,
    )?;
    Ok(broker_result_envelope(
        &prepared.ids,
        json!({
            "action": "preview",
            "security": prepared.security,
            "input": requested_savings_plan_input(&args, prepared.isin.as_str(), prepared.amount.as_str()),
            "effective_configuration": prepared.effective_configuration,
            "cost_venue": SAVINGS_PLAN_EXECUTION_VENUE,
            "ex_ante_costs": prepared.costs,
            "confirmation": confirmation_payload,
            "compliance": savings_plan_compliance_payload(),
            "presentation": presentation,
            "next_step": "confirm_with_id",
        }),
    ))
}

fn execute_broker_savings_plan_add_phase2(
    args: crate::cli::BrokerSavingsPlanAddArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let confirmation_id = args
        .confirm
        .as_deref()
        .ok_or_else(|| anyhow!("SAVINGS_PLAN_CONFIRMATION_NOT_FOUND: missing confirmation id"))?;
    let now_epoch = current_epoch_seconds();
    let stored = load_savings_plan_confirmation(confirmation_id, now_epoch)?;
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    if stored.env != env.as_str() {
        bail!(
            "SAVINGS_PLAN_CONFIRMATION_ENV_MISMATCH: confirmation env '{}' does not match active env '{}'",
            stored.env,
            env.as_str()
        );
    }
    let env_cfg = crate::channel::current_env_config();
    let loaded_session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let mut session = loaded_session.session;
    let access_context = loaded_session.access_context;
    let prepared = prepare_savings_plan_add(
        &args,
        session_manager,
        env,
        &env_cfg,
        &mut session,
        access_context,
        dpop_options,
    )?;
    ensure_phase2_savings_plan_matches(&stored, &prepared, env)?;
    let mutation_variables = broker_create_or_update_savings_plan_variables(
        &prepared.ids.portfolio_id,
        &prepared.isin,
        &prepared.amount,
        prepared
            .effective_configuration
            .get("frequency")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        prepared
            .effective_configuration
            .get("day_of_month")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .ok_or_else(|| {
                anyhow!("SAVINGS_PLAN_CONFIRMATION_FIELDS_MISMATCH: invalid effective day")
            })?,
        prepared
            .effective_configuration
            .get("year_month")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        prepared
            .effective_configuration
            .get("dynamization_rate")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        prepared
            .effective_configuration
            .get("payment_method")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        args.appropriateness_id.as_deref(),
        args.acknowledged_appropriateness_warning_version.as_deref(),
    )
    .map_err(|error| anyhow!("SAVINGS_PLAN_INPUT_INVALID: {error}"))?;
    enforce_graphql_access_policy(
        BROKER_CREATE_OR_UPDATE_SAVINGS_PLAN_MUTATION,
        Some("BrokerCreateOrUpdateSavingsPlan"),
        access_context,
    )?;
    let started = start_submission(
        confirmation_id,
        stored.snapshot_checksum.as_str(),
        current_epoch_seconds(),
    )?;
    let mutation_response = execute_graphql_once_with_headers(
        &env_cfg.graphql_url,
        &session.access_token,
        BROKER_CREATE_OR_UPDATE_SAVINGS_PLAN_MUTATION,
        &mutation_variables,
        Some("BrokerCreateOrUpdateSavingsPlan"),
        access_context,
        &[("X-SC-Order-Origin", "CLI")],
        dpop_options,
    );
    let mutation_projected = match mutation_response {
        Ok(response) => match project_broker_create_or_update_savings_plan_response(&response) {
            Ok(projected) => projected,
            Err(error) => return savings_plan_unknown_submission_error(&started, error),
        },
        Err(error) if is_definitive_savings_plan_rejection(&error) => {
            finalize_consumed(&started, current_epoch_seconds()).map_err(|_| {
                anyhow!("SAVINGS_PLAN_SUBMISSION_UNKNOWN: backend rejected the submission but local state could not be finalized; inspect `sc broker savings-plans`")
            })?;
            return Err(anyhow!("SAVINGS_PLAN_SUBMISSION_FAILED: {error}"));
        }
        Err(error) => return savings_plan_unknown_submission_error(&started, error),
    };
    finalize_consumed(&started, current_epoch_seconds()).map_err(|_| {
        anyhow!("SAVINGS_PLAN_SUBMISSION_UNKNOWN: mutation acknowledgement was received but local state could not be finalized; inspect `sc broker savings-plans`")
    })?;

    let input = validated_broker_input(&prepared.ids, false, None)?;
    let readback_variables = broker_savings_plan_by_isin_variables(&input, &prepared.isin)
        .map_err(|error| anyhow!("SAVINGS_PLAN_INPUT_INVALID: {error}"))?;
    let readback = execute_with_refresh_retry(
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
    )
    .and_then(|response| project_broker_savings_plan_by_isin_response(&response));
    let (security, savings_plan, warning) = match readback {
        Ok(projected) => {
            let savings_plan = projected
                .get("savings_plan")
                .cloned()
                .unwrap_or(Value::Null);
            let warning = if savings_plan.is_null() {
                Value::String(
                    "mutation acknowledged but no active savings plan returned in readback"
                        .to_string(),
                )
            } else {
                Value::Null
            };
            (
                projected.get("security").cloned().unwrap_or(Value::Null),
                savings_plan,
                warning,
            )
        }
        Err(error) => (
            Value::Null,
            Value::Null,
            Value::String(format!(
                "mutation acknowledged; readback failed: {}",
                crate::user_error_message(&error)
            )),
        ),
    };
    Ok(broker_result_envelope(
        &prepared.ids,
        json!({
            "action": "create_or_update",
            "security": security,
            "input": requested_savings_plan_input(&args, prepared.isin.as_str(), prepared.amount.as_str()),
            "effective_configuration": prepared.effective_configuration,
            "mutation_id": mutation_projected.get("mutation_id").cloned().unwrap_or(Value::Null),
            "savings_plan": savings_plan,
            "warning": warning,
            "confirmation": {"id": confirmation_id, "consumed": true},
            "next_step": "completed",
        }),
    ))
}

#[allow(clippy::too_many_arguments)]
fn prepare_savings_plan_add(
    args: &crate::cli::BrokerSavingsPlanAddArgs,
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    session: &mut Session,
    access_context: crate::graphql::GraphqlAccessContext,
    dpop_options: &crate::dpop::DpopRuntimeOptions,
) -> Result<PreparedSavingsPlanAdd> {
    let loaded_config = load_broker_savings_plan_config_response_with_session(
        session_manager,
        env,
        env_cfg,
        session,
        access_context,
        dpop_options,
        args.portfolio_id.as_deref(),
        args.isin.as_str(),
    )?;
    let LoadedBrokerSavingsPlanConfig {
        ids,
        requested_isin: isin,
        response: config_response,
    } = loaded_config;
    let amount = normalize_positive_decimal_for_savings(args.amount.as_str(), "amount")?;
    let amount_value = parse_positive_decimal_for_savings(amount.as_str(), "amount")?;
    let config_details = project_broker_savings_plan_config_details_response(&isin, &config_response)
        .map_err(|_| anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: savings plan is not available for this instrument in the selected portfolio context"))?;
    let config_projected = config_details
        .get("savings_plan_configuration")
        .cloned()
        .ok_or_else(|| {
            anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: savings plan configuration is unavailable")
        })?;
    let parsed_config = parse_broker_savings_plan_config(&config_projected).map_err(|_| {
        anyhow!(
            "SAVINGS_PLAN_CONFIG_UNAVAILABLE: savings plan is not available for this instrument in the selected portfolio context"
        )
    })?;
    validate_amount_against_config(amount_value, &parsed_config)?;
    let effective = resolve_effective_savings_plan_add_config(args, &parsed_config)?;
    let effective_configuration = effective_savings_plan_config_value(&effective);
    let input = validated_broker_input(&ids, false, None)?;
    let cost_variables = broker_savings_plan_ex_ante_cost_variables(
        &input,
        &isin,
        effective.frequency.as_str(),
        &amount,
    )
    .map_err(|error| anyhow!("SAVINGS_PLAN_INPUT_INVALID: {error}"))?;
    let costs_response = execute_with_refresh_retry(
        session_manager,
        env,
        env_cfg,
        session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                BROKER_SAVINGS_PLAN_EX_ANTE_COSTS_QUERY,
                &cost_variables,
                Some("BrokerSavingsPlanExAnteCost"),
                access_context,
                dpop_options,
            )
        },
    )
    .map_err(|error| anyhow!("SAVINGS_PLAN_EX_ANTE_COST_UNAVAILABLE: {error}"))?;
    let costs = project_broker_savings_plan_ex_ante_costs_response(&costs_response)?;
    validate_savings_plan_ex_ante_costs(&costs)?;
    let security = config_details
        .get("security")
        .cloned()
        .unwrap_or(Value::Null);
    let snapshot = json!({
        "env": env.as_str(),
        "account_id": ids.account_id,
        "portfolio_id": ids.portfolio_id,
        "input": requested_savings_plan_input(args, isin.as_str(), amount.as_str()),
        "effective_configuration": effective_configuration,
        "cost_venue": SAVINGS_PLAN_EXECUTION_VENUE,
        "ex_ante_costs": costs,
    });
    let snapshot_checksum = checksum_for_payload(&snapshot);
    Ok(PreparedSavingsPlanAdd {
        ids,
        isin,
        amount,
        effective_configuration,
        security,
        costs,
        snapshot,
        snapshot_checksum,
    })
}

fn effective_savings_plan_config_value(config: &ResolvedSavingsPlanAddConfig) -> Value {
    json!({
        "frequency": config.frequency,
        "day_of_month": config.day_of_month,
        "year_month": config.year_month,
        "dynamization_rate": config.dynamization_rate,
        "payment_method": config.payment_method,
    })
}

fn requested_savings_plan_input(
    args: &crate::cli::BrokerSavingsPlanAddArgs,
    normalized_isin: &str,
    amount: &str,
) -> Value {
    json!({
        "isin": normalized_isin,
        "amount": amount,
        "frequency": args.frequency.map(|value| value.as_graphql()),
        "day_of_month": args.day_of_month,
        "year_month": normalized_optional_savings_plan_arg(args.year_month.as_deref()),
        "dynamization_rate": normalized_optional_savings_plan_arg(args.dynamization_rate.as_deref()),
        "payment_method": args.payment_method.map(|value| value.as_graphql()),
        "appropriateness_id": normalized_optional_savings_plan_arg(args.appropriateness_id.as_deref()),
        "acknowledged_appropriateness_warning_version": normalized_optional_savings_plan_arg(args.acknowledged_appropriateness_warning_version.as_deref()),
    })
}

fn normalized_optional_savings_plan_arg(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn ensure_phase2_savings_plan_matches(
    stored: &SavingsPlanConfirmation,
    prepared: &PreparedSavingsPlanAdd,
    env: TargetEnv,
) -> Result<()> {
    if stored.account_id != prepared.ids.account_id {
        bail!(
            "SAVINGS_PLAN_CONFIRMATION_ACCOUNT_MISMATCH: confirmation account '{}' does not match active account '{}'",
            stored.account_id,
            prepared.ids.account_id
        );
    }
    if stored.portfolio_id != prepared.ids.portfolio_id {
        bail!(
            "SAVINGS_PLAN_CONFIRMATION_PORTFOLIO_MISMATCH: confirmation portfolio '{}' does not match active portfolio '{}'",
            stored.portfolio_id,
            prepared.ids.portfolio_id
        );
    }
    if stored.env != env.as_str() || stored.snapshot_checksum != prepared.snapshot_checksum {
        bail!(
            "SAVINGS_PLAN_CONFIRMATION_FIELDS_MISMATCH: savings-plan configuration or disclosed costs changed; rerun phase 1"
        );
    }
    Ok(())
}

fn savings_plan_unknown_submission_error<T>(
    started: &crate::savings_plan_confirmation::StartedSavingsPlanSubmission,
    error: anyhow::Error,
) -> Result<T> {
    let _ = finalize_unknown(started, current_epoch_seconds());
    Err(anyhow!(
        "SAVINGS_PLAN_SUBMISSION_UNKNOWN: {}. Inspect `sc broker savings-plans` for the matching account and portfolio before creating a new preview",
        crate::user_error_message(&error)
    ))
}

fn is_definitive_savings_plan_rejection(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("SAVINGS_PLAN_INPUT_INVALID:")
        || message.contains(
            "GraphQL returned errors for BrokerCreateOrUpdateSavingsPlan (code: BAD_USER_INPUT)",
        )
}

fn savings_plan_compliance_payload() -> Value {
    json!({
        "rule_id": SAVINGS_PLAN_COMPLIANCE_RULE_ID,
        "must_present_all_information": true,
        "instruction": "Before running phase 2, you MUST present all phase-1 savings-plan information in a human-readable summary without omitting or changing values. Then obtain an explicit affirmative confirmation in a separate interaction. You MUST NOT execute phase 2 automatically, implicitly, or in the same step as phase 1 output.",
        "requires_explicit_user_confirmation_between_phases": true,
        "forbid_automatic_phase_2_execution": true,
        "confirmation_must_be_separate_step": true,
        "presentation": {
            "format": SAVINGS_PLAN_PRESENTATION_FORMAT,
            "section_order": ["savings_plan", "ex_ante_costs", "confirmation"],
            "required_leaf_paths": savings_plan_required_leaf_paths(),
            "preserve_exact_values": true,
            "display_null_as_literal": true,
            "raw_json_only_on_user_request": true,
        }
    })
}

fn savings_plan_command_template(
    args: &crate::cli::BrokerSavingsPlanAddArgs,
    confirmation_id: &str,
    json_mode: bool,
) -> String {
    let mut command = format!(
        "sc broker savings-plans add --isin {} --amount {}",
        args.isin.trim(),
        args.amount.trim()
    );
    if let Some(portfolio_id) = normalized_optional_savings_plan_arg(args.portfolio_id.as_deref()) {
        command.push_str(&format!(" --portfolio-id {portfolio_id}"));
    }
    if let Some(frequency) = args.frequency {
        let frequency = frequency
            .to_possible_value()
            .map(|value| value.get_name().to_string())
            .unwrap_or_default();
        command.push_str(&format!(" --frequency {frequency}"));
    }
    if let Some(day) = args.day_of_month {
        command.push_str(&format!(" --day-of-month {day}"));
    }
    for (flag, value) in [
        ("year-month", args.year_month.as_deref()),
        ("dynamization-rate", args.dynamization_rate.as_deref()),
        ("appropriateness-id", args.appropriateness_id.as_deref()),
        (
            "acknowledged-appropriateness-warning-version",
            args.acknowledged_appropriateness_warning_version.as_deref(),
        ),
    ] {
        if let Some(value) = normalized_optional_savings_plan_arg(value) {
            command.push_str(&format!(" --{flag} {value}"));
        }
    }
    if let Some(payment_method) = args.payment_method {
        let payment_method = payment_method
            .to_possible_value()
            .map(|value| value.get_name().to_string())
            .unwrap_or_default();
        command.push_str(&format!(" --payment-method {payment_method}"));
    }
    command.push_str(&format!(" --confirm {confirmation_id}"));
    if json_mode {
        command.push_str(" --json");
    }
    command
}

fn validate_savings_plan_ex_ante_costs(costs: &Value) -> Result<()> {
    let object = costs
        .as_object()
        .ok_or_else(|| unavailable_cost_error("cost payload must be an object"))?;
    validate_cost_id(object.get("id"))?;
    for group in ["entryCosts", "ongoingCosts", "exitCosts"] {
        validate_cost_group(
            object.get(group),
            &["productCosts", "serviceCosts", "total"],
        )?;
    }
    validate_cost_group(
        object.get("effectOnReturn"),
        &["initialYearCosts", "followingYearsCosts", "finalYearCosts"],
    )?;
    for group in ["fiveYearsCosts", "incidentalCosts"] {
        validate_cost_leaf_group(object.get(group))?;
    }
    Ok(())
}

fn validate_cost_id(value: Option<&Value>) -> Result<()> {
    match value {
        Some(Value::String(_)) | Some(Value::Null) => Ok(()),
        _ => Err(unavailable_cost_error("cost payload has invalid id")),
    }
}

fn validate_cost_group(value: Option<&Value>, fields: &[&str]) -> Result<()> {
    let Some(value) = value else {
        return Err(unavailable_cost_error("cost payload is incomplete"));
    };
    if value.is_null() {
        return Ok(());
    }
    let object = value
        .as_object()
        .ok_or_else(|| unavailable_cost_error("cost group must be an object or null"))?;
    for field in fields {
        validate_cost_leaf_group(object.get(*field))?;
    }
    Ok(())
}

fn validate_cost_leaf_group(value: Option<&Value>) -> Result<()> {
    let Some(value) = value else {
        return Err(unavailable_cost_error("cost leaf group is missing"));
    };
    if value.is_null() {
        return Ok(());
    }
    let object = value
        .as_object()
        .ok_or_else(|| unavailable_cost_error("cost leaf group must be an object or null"))?;
    for field in ["amount", "percentage"] {
        match object.get(field) {
            Some(Value::String(_)) | Some(Value::Number(_)) | Some(Value::Null) => {}
            _ => {
                return Err(unavailable_cost_error(
                    "cost amount or percentage is missing or malformed",
                ));
            }
        }
    }
    Ok(())
}

fn unavailable_cost_error(detail: &str) -> anyhow::Error {
    anyhow!("SAVINGS_PLAN_EX_ANTE_COST_UNAVAILABLE: {detail}")
}

fn current_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
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

    Ok(broker_result_envelope(&ids, projected))
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

fn resolve_default_savings_plan_config(
    config: &ParsedBrokerSavingsPlanConfig,
) -> Result<ResolvedSavingsPlanAddConfig> {
    let frequency = if config.frequencies.iter().any(|f| f == "MONTHLY") {
        "MONTHLY".to_string()
    } else {
        config.frequencies[0].clone()
    };

    let selected_schedule = config
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
        })?;

    let year_month = selected_schedule
        .available_year_months
        .iter()
        .min()
        .cloned()
        .ok_or_else(|| {
            anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: no available yearMonth for selected schedule")
        })?;

    let dynamization_rate = normalize_non_negative_decimal_for_savings(
        config.default_dynamization_rate.as_str(),
        "dynamization_rate",
    )?;

    let payment_method = if config
        .payment_methods
        .iter()
        .any(|method| method == "REFERENCE_ACCOUNT")
    {
        "REFERENCE_ACCOUNT".to_string()
    } else {
        config.payment_methods[0].clone()
    };

    Ok(ResolvedSavingsPlanAddConfig {
        frequency,
        day_of_month: selected_schedule.day_of_month,
        year_month,
        dynamization_rate,
        payment_method,
    })
}

fn project_public_broker_savings_plan_config_response(
    requested_isin: &str,
    projected: &Value,
    config: &ParsedBrokerSavingsPlanConfig,
    defaults: &ResolvedSavingsPlanAddConfig,
) -> Value {
    let mut schedules = config.schedules.clone();
    schedules.sort_by_key(|schedule| schedule.day_of_month);

    json!({
        "security": {
            "isin": projected
                .get("security")
                .and_then(|security| security.get("isin"))
                .cloned()
                .unwrap_or_else(|| Value::String(requested_isin.to_string())),
            "name": projected
                .get("security")
                .and_then(|security| security.get("name"))
                .cloned()
                .unwrap_or(Value::Null),
            "security_type": projected
                .get("security")
                .and_then(|security| security.get("security_type"))
                .cloned()
                .unwrap_or(Value::Null),
        },
        "amount_limits": {
            "min": config.min_amount.to_string(),
            "max": config.max_amount.to_string(),
        },
        "defaults": {
            "frequency": defaults.frequency.clone(),
            "day_of_month": defaults.day_of_month,
            "year_month": defaults.year_month.clone(),
            "dynamization_rate": defaults.dynamization_rate.clone(),
            "payment_method": defaults.payment_method.clone(),
        },
        "frequencies": config.frequencies.clone(),
        "payment_methods": config.payment_methods.clone(),
        "dynamization_rates": config.dynamization_rates.clone(),
        "schedules": schedules.iter().map(|schedule| {
            json!({
                "day_of_month": schedule.day_of_month,
                "is_default": schedule.is_default,
                "is_earliest": schedule.is_earliest,
                "available_year_months": schedule.available_year_months.clone(),
            })
        }).collect::<Vec<_>>(),
    })
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
    let defaults = resolve_default_savings_plan_config(config)?;

    let frequency = match args.frequency {
        Some(value) => {
            let gql = value.as_graphql().to_string();
            if !config.frequencies.iter().any(|f| f == &gql) {
                bail!("SAVINGS_PLAN_INPUT_INVALID: field 'frequency' is not allowed");
            }
            gql
        }
        None => defaults.frequency.clone(),
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
            .find(|schedule| schedule.day_of_month == defaults.day_of_month)
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
        None => defaults.year_month.clone(),
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
        None => defaults.dynamization_rate.clone(),
    };

    let payment_method = match args.payment_method {
        Some(value) => {
            let gql = value.as_graphql().to_string();
            if !config.payment_methods.iter().any(|method| method == &gql) {
                bail!("SAVINGS_PLAN_INPUT_INVALID: field 'payment_method' is not allowed");
            }
            gql
        }
        None => defaults.payment_method.clone(),
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
            confirm: None,
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

    fn sample_savings_plan_costs() -> Value {
        json!({
            "id": "cost-1",
            "entryCosts": {
                "productCosts": {"amount": "1", "percentage": "0.1"},
                "serviceCosts": {"amount": "2", "percentage": "0.2"},
                "total": {"amount": "3", "percentage": "0.3"}
            },
            "ongoingCosts": {
                "productCosts": {"amount": "1", "percentage": "0.1"},
                "serviceCosts": {"amount": "2", "percentage": "0.2"},
                "total": {"amount": "3", "percentage": "0.3"}
            },
            "exitCosts": {
                "productCosts": {"amount": "1", "percentage": "0.1"},
                "serviceCosts": {"amount": "2", "percentage": "0.2"},
                "total": {"amount": "3", "percentage": "0.3"}
            },
            "effectOnReturn": {
                "initialYearCosts": {"amount": "1", "percentage": "0.1"},
                "followingYearsCosts": {"amount": "2", "percentage": "0.2"},
                "finalYearCosts": {"amount": "3", "percentage": "0.3"}
            },
            "fiveYearsCosts": {"amount": "4", "percentage": "0.4"},
            "incidentalCosts": null
        })
    }

    fn sample_savings_plan_config_response() -> &'static str {
        r#"{
            "data": {
                "account": {
                    "brokerPortfolio": {
                        "security": {
                            "isin": "US0378331005",
                            "name": "Apple Inc.",
                            "type": "STOCK",
                            "savingsPlanConfiguration": {
                                "schedules": [{
                                    "dayOfTheMonth": 5,
                                    "isEarliest": true,
                                    "isDefault": true,
                                    "yearMonths": [{ "yearMonth": "2026-04", "isAvailable": true }]
                                }],
                                "minSavingsPlanAmount": "1",
                                "maxSavingsPlanAmount": "10000",
                                "dynamizationRates": [0, 1.5],
                                "defaultDynamizationRate": 0,
                                "paymentMethods": ["REFERENCE_ACCOUNT"],
                                "frequencies": ["MONTHLY"]
                            }
                        }
                    }
                }
            }
        }"#
    }

    fn savings_plan_cost_response(costs: Value) -> String {
        json!({
            "data": {
                "account": {
                    "brokerPortfolio": {
                        "savingsPlanExAnteCosts": costs,
                    }
                }
            }
        })
        .to_string()
    }

    fn save_active_test_session(session_manager: &mut SessionManager, config: &AppConfig) {
        session_manager
            .save_active(&StoredSession {
                env: crate::channel::current_env(),
                session: sample_session(),
                dpop_jwk_thumbprint: Some(current_runtime_dpop_thumbprint(config)),
                mode: None,
            })
            .expect("save session");
    }

    #[test]
    fn strict_savings_plan_cost_validation_preserves_explicit_nulls_and_rejects_missing_leaves() {
        validate_savings_plan_ex_ante_costs(&sample_savings_plan_costs())
            .expect("explicit null cost group is valid");

        let mut malformed = sample_savings_plan_costs();
        malformed["entryCosts"]["total"] = json!({"amount": "3"});
        let err = validate_savings_plan_ex_ante_costs(&malformed).expect_err("missing percentage");
        assert!(
            err.to_string()
                .contains("SAVINGS_PLAN_EX_ANTE_COST_UNAVAILABLE:")
        );
    }

    #[test]
    fn savings_plan_preview_returns_full_cost_disclosure_and_never_submits() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_authorization_header())
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanConfig".to_string(),
            ))
            .match_body(mockito::Matcher::PartialJson(json!({
                "variables": {
                    "accountId": "person-1",
                    "portfolioId": "portfolio-1",
                    "isin": "US0378331005"
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(sample_savings_plan_config_response())
            .expect(1)
            .create();
        let costs = sample_savings_plan_costs();
        let costs_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_authorization_header())
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanExAnteCost".to_string(),
            ))
            .match_body(mockito::Matcher::PartialJson(json!({
                "variables": {
                    "accountId": "person-1",
                    "portfolioId": "portfolio-1",
                    "isin": "US0378331005",
                    "frequency": "MONTHLY",
                    "amount": "100",
                    "venue": "SEIX"
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(savings_plan_cost_response(costs.clone()))
            .expect(1)
            .create();
        let mutation_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex(
                "BrokerCreateOrUpdateSavingsPlan".to_string(),
            ))
            .expect(0)
            .create();

        let config = sample_runtime_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        save_active_test_session(&mut session_manager, &config);
        let mut args = sample_args();
        args.portfolio_id = Some("portfolio-1".to_string());
        args.isin = "us0378331005".to_string();

        let payload = execute_broker_savings_plan_add(args, &config, &mut session_manager)
            .expect("preview payload");
        let result = &payload["result"];
        let confirmation_id = result["confirmation"]["id"]
            .as_str()
            .expect("confirmation id");

        assert_eq!(result["action"], "preview");
        assert_eq!(result["input"]["isin"], "US0378331005");
        assert_eq!(result["cost_venue"], "SEIX");
        assert_eq!(result["presentation"]["savings_plan"]["cost_venue"], "SEIX");
        assert_eq!(result["ex_ante_costs"], costs);
        assert_eq!(result["next_step"], "confirm_with_id");
        assert!(confirmation_id.starts_with("scsp1_"));
        assert!(
            result["presentation"]["required_leaf_paths"]
                .as_array()
                .expect("paths")
                .iter()
                .any(|path| path == "ex_ante_costs.entryCosts.total.percentage")
        );
        let stored = load_savings_plan_confirmation(confirmation_id, current_epoch_seconds())
            .expect("stored confirmation");
        assert_eq!(stored.snapshot["input"]["isin"], "US0378331005");
        assert_eq!(stored.snapshot["cost_venue"], "SEIX");

        config_mock.assert();
        costs_mock.assert();
        mutation_mock.assert();
    }

    #[test]
    fn malformed_preview_costs_leave_an_existing_confirmation_unchanged_and_do_not_submit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let now_epoch = current_epoch_seconds();
        let existing_snapshot = json!({"existing": true});
        let existing = SavingsPlanConfirmation {
            confirmation_id: "scsp1_existing".to_string(),
            snapshot_checksum: checksum_for_payload(&existing_snapshot),
            created_at_epoch: now_epoch,
            expires_at_epoch: now_epoch + 900,
            env: crate::channel::current_env().as_str().to_string(),
            account_id: "person-1".to_string(),
            portfolio_id: "portfolio-1".to_string(),
            isin: "US0378331005".to_string(),
            snapshot: existing_snapshot,
        };
        store_pending(existing).expect("store existing confirmation");
        let config_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanConfig".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(sample_savings_plan_config_response())
            .expect(1)
            .create();
        let mut malformed_costs = sample_savings_plan_costs();
        malformed_costs["entryCosts"]["total"] = json!({"amount": "3"});
        let costs_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanExAnteCost".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(savings_plan_cost_response(malformed_costs))
            .expect(1)
            .create();
        let mutation_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex(
                "BrokerCreateOrUpdateSavingsPlan".to_string(),
            ))
            .expect(0)
            .create();

        let config = sample_runtime_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        save_active_test_session(&mut session_manager, &config);
        let mut args = sample_args();
        args.portfolio_id = Some("portfolio-1".to_string());

        let err = execute_broker_savings_plan_add(args, &config, &mut session_manager)
            .expect_err("malformed costs must fail closed");
        assert!(
            err.to_string()
                .contains("SAVINGS_PLAN_EX_ANTE_COST_UNAVAILABLE:")
        );
        let stored = load_savings_plan_confirmation("scsp1_existing", current_epoch_seconds())
            .expect("existing confirmation remains");
        assert_eq!(stored.snapshot, json!({"existing": true}));

        config_mock.assert();
        costs_mock.assert();
        mutation_mock.assert();
    }

    #[test]
    fn savings_plan_confirmation_submits_one_mutation_after_fresh_preview_validation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanConfig".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(sample_savings_plan_config_response())
            .expect(2)
            .create();
        let costs_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanExAnteCost".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(savings_plan_cost_response(sample_savings_plan_costs()))
            .expect(2)
            .create();
        let mutation_mock = server
            .mock("POST", "/")
            .match_header("x-sc-order-origin", "CLI")
            .match_body(mockito::Matcher::Regex(
                "BrokerCreateOrUpdateSavingsPlan".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"createOrUpdateSavingsPlan":{"id":"mutation-1"}}}"#)
            .expect(1)
            .create();
        let readback_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex("BrokerSavingsPlanByIsin".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{
                "data": {
                    "account": {
                        "brokerPortfolio": {
                            "security": {
                                "isin": "US0378331005",
                                "name": "Apple Inc.",
                                "type": "STOCK",
                                "inventory": {
                                    "savingsPlan": {
                                        "isin": "US0378331005",
                                        "amount": "100",
                                        "frequency": "MONTHLY",
                                        "dayOfTheMonth": 5,
                                        "dynamizationRate": "0",
                                        "paymentMethod": "REFERENCE_ACCOUNT",
                                        "nextExecutionDate": {"date": "2026-04-05", "epochDay": 20548}
                                    }
                                }
                            }
                        }
                    }
                }
            }"#)
            .expect(1)
            .create();

        let config = sample_runtime_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        save_active_test_session(&mut session_manager, &config);
        let mut phase1 = sample_args();
        phase1.portfolio_id = Some("portfolio-1".to_string());
        let preview = execute_broker_savings_plan_add(phase1, &config, &mut session_manager)
            .expect("preview");
        let confirmation_id = preview["result"]["confirmation"]["id"]
            .as_str()
            .expect("confirmation id")
            .to_string();
        let mut phase2 = sample_args();
        phase2.portfolio_id = Some("portfolio-1".to_string());
        phase2.confirm = Some(confirmation_id.clone());

        let payload = execute_broker_savings_plan_add(phase2, &config, &mut session_manager)
            .expect("confirmation submission");
        assert_eq!(payload["result"]["action"], "create_or_update");
        assert_eq!(payload["result"]["mutation_id"], "mutation-1");
        assert_eq!(
            payload["result"]["confirmation"],
            json!({"id": confirmation_id, "consumed": true})
        );
        assert_eq!(payload["result"]["savings_plan"]["amount"], "100");
        let consumed = load_savings_plan_confirmation(
            payload["result"]["confirmation"]["id"]
                .as_str()
                .expect("confirmation id"),
            current_epoch_seconds(),
        )
        .expect_err("consumed confirmation cannot be reused");
        assert!(
            consumed
                .to_string()
                .contains("SAVINGS_PLAN_CONFIRMATION_ALREADY_USED:")
        );

        config_mock.assert();
        costs_mock.assert();
        mutation_mock.assert();
        readback_mock.assert();
    }

    #[test]
    fn savings_plan_malformed_acknowledgement_is_not_retried_and_blocks_new_previews() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanConfig".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(sample_savings_plan_config_response())
            .expect(2)
            .create();
        let costs_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanExAnteCost".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(savings_plan_cost_response(sample_savings_plan_costs()))
            .expect(2)
            .create();
        let mutation_mock = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex(
                "BrokerCreateOrUpdateSavingsPlan".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"createOrUpdateSavingsPlan":null}}"#)
            .expect(1)
            .create();

        let config = sample_runtime_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        save_active_test_session(&mut session_manager, &config);
        let mut phase1 = sample_args();
        phase1.portfolio_id = Some("portfolio-1".to_string());
        let preview = execute_broker_savings_plan_add(phase1, &config, &mut session_manager)
            .expect("preview");
        let confirmation_id = preview["result"]["confirmation"]["id"]
            .as_str()
            .expect("confirmation id")
            .to_string();
        let mut phase2 = sample_args();
        phase2.portfolio_id = Some("portfolio-1".to_string());
        phase2.confirm = Some(confirmation_id.clone());

        let err = execute_broker_savings_plan_add(phase2, &config, &mut session_manager)
            .expect_err("malformed acknowledgement leaves mutation outcome unknown");
        assert!(err.to_string().contains("SAVINGS_PLAN_SUBMISSION_UNKNOWN:"));
        let mut retry_phase2 = sample_args();
        retry_phase2.portfolio_id = Some("portfolio-1".to_string());
        retry_phase2.confirm = Some(confirmation_id);
        let retry_err =
            execute_broker_savings_plan_add(retry_phase2, &config, &mut session_manager)
                .expect_err("the same confirmation must not resend a mutation");
        assert!(
            retry_err
                .to_string()
                .contains("SAVINGS_PLAN_SUBMISSION_UNKNOWN:")
        );
        assert!(assert_preview_allowed().is_err());

        config_mock.assert();
        costs_mock.assert();
        mutation_mock.assert();
    }

    #[test]
    fn savings_plan_only_treats_explicit_validation_errors_as_definitive_rejections() {
        assert!(is_definitive_savings_plan_rejection(&anyhow!(
            "GraphQL returned errors for BrokerCreateOrUpdateSavingsPlan (code: BAD_USER_INPUT)"
        )));
        assert!(!is_definitive_savings_plan_rejection(&anyhow!(
            "GraphQL returned errors for BrokerCreateOrUpdateSavingsPlan (code: INTERNAL_SERVER_ERROR)"
        )));
    }

    #[test]
    fn savings_plan_phase2_template_keeps_confirm_before_json() {
        let args = sample_args();
        let template = savings_plan_command_template(&args, "scsp1_confirm", true);
        assert!(template.contains("--confirm scsp1_confirm --json"));
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
    fn resolve_default_savings_plan_config_uses_reference_account_and_sorted_month() {
        let mut config = sample_config();
        config.schedules[0].available_year_months = vec![
            "2026-08".to_string(),
            "2026-06".to_string(),
            "2026-07".to_string(),
        ];

        let resolved = resolve_default_savings_plan_config(&config).expect("resolve");

        assert_eq!(resolved.frequency, "MONTHLY");
        assert_eq!(resolved.day_of_month, 5);
        assert_eq!(resolved.year_month, "2026-06");
        assert_eq!(resolved.dynamization_rate, "0");
        assert_eq!(resolved.payment_method, "REFERENCE_ACCOUNT");
    }

    #[test]
    fn resolve_default_savings_plan_config_falls_back_without_monthly_or_reference_account() {
        let mut config = sample_config();
        config.frequencies = vec!["QUARTERLY".to_string(), "ANNUALLY".to_string()];
        config.payment_methods = vec!["CASH_BALANCE".to_string()];
        config.schedules = vec![
            ParsedBrokerSavingsPlanSchedule {
                day_of_month: 9,
                is_default: false,
                is_earliest: true,
                available_year_months: vec!["2026-09".to_string()],
            },
            ParsedBrokerSavingsPlanSchedule {
                day_of_month: 4,
                is_default: false,
                is_earliest: false,
                available_year_months: vec!["2026-08".to_string()],
            },
        ];

        let resolved = resolve_default_savings_plan_config(&config).expect("resolve");

        assert_eq!(resolved.frequency, "QUARTERLY");
        assert_eq!(resolved.day_of_month, 9);
        assert_eq!(resolved.payment_method, "CASH_BALANCE");
    }

    #[test]
    fn resolve_default_savings_plan_config_falls_back_to_lowest_day_when_no_flags_exist() {
        let mut config = sample_config();
        config.schedules = vec![
            ParsedBrokerSavingsPlanSchedule {
                day_of_month: 12,
                is_default: false,
                is_earliest: false,
                available_year_months: vec!["2026-12".to_string()],
            },
            ParsedBrokerSavingsPlanSchedule {
                day_of_month: 3,
                is_default: false,
                is_earliest: false,
                available_year_months: vec!["2026-03".to_string()],
            },
        ];

        let resolved = resolve_default_savings_plan_config(&config).expect("resolve");

        assert_eq!(resolved.day_of_month, 3);
        assert_eq!(resolved.year_month, "2026-03");
    }

    #[test]
    fn project_public_broker_savings_plan_config_response_maps_public_shape() {
        let projected = json!({
            "security": {
                "isin": "US0378331005",
                "name": "Apple Inc.",
                "security_type": "STOCK"
            }
        });
        let config = sample_config();
        let defaults = resolve_default_savings_plan_config(&config).expect("resolve");

        let public = project_public_broker_savings_plan_config_response(
            "US0378331005",
            &projected,
            &config,
            &defaults,
        );

        assert_eq!(public["security"]["isin"], "US0378331005");
        assert_eq!(public["security"]["name"], "Apple Inc.");
        assert_eq!(public["security"]["security_type"], "STOCK");
        assert_eq!(public["amount_limits"]["min"], "1");
        assert_eq!(public["amount_limits"]["max"], "10000");
        assert_eq!(public["defaults"]["frequency"], "MONTHLY");
        assert_eq!(public["defaults"]["payment_method"], "REFERENCE_ACCOUNT");
        assert_eq!(public["schedules"][0]["day_of_month"], 5);
        assert_eq!(
            public["schedules"][0]["available_year_months"][0],
            "2026-04"
        );
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
            trade_controls: None,
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
    fn execute_broker_savings_plan_config_returns_public_result_envelope() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());

        let config_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_authorization_header())
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanConfig".to_string(),
            ))
            .match_body(mockito::Matcher::PartialJson(json!({
                "variables": {
                    "accountId": "person-1",
                    "portfolioId": "portfolio-1",
                    "isin": "US0378331005"
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "account": {
                            "brokerPortfolio": {
                                "security": {
                                    "isin": "US0378331005",
                                    "name": "Apple Inc.",
                                    "type": "STOCK",
                                    "savingsPlanConfiguration": {
                                        "schedules": [
                                            {
                                                "dayOfTheMonth": 1,
                                                "isEarliest": true,
                                                "isDefault": true,
                                                "yearMonths": [
                                                    { "yearMonth": "2026-07", "isAvailable": true },
                                                    { "yearMonth": "2026-08", "isAvailable": true }
                                                ]
                                            }
                                        ],
                                        "minSavingsPlanAmount": "25",
                                        "maxSavingsPlanAmount": "5000",
                                        "defaultMinSavingsPlanAmount": "25",
                                        "dynamizationRates": [0, 1.5],
                                        "defaultDynamizationRate": 0,
                                        "paymentMethods": ["CASH_BALANCE", "REFERENCE_ACCOUNT"],
                                        "frequencies": ["QUARTERLY", "MONTHLY"]
                                    }
                                }
                            }
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

        let payload = execute_broker_savings_plan_config(
            crate::cli::BrokerSavingsPlanConfigArgs {
                portfolio_id: Some("portfolio-1".to_string()),
                isin: "us0378331005".to_string(),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect("config payload");

        assert_eq!(payload["account_id"], "person-1");
        assert_eq!(payload["portfolio_id"], "portfolio-1");
        assert_eq!(payload["result"]["security"]["name"], "Apple Inc.");
        assert_eq!(payload["result"]["amount_limits"]["min"], "25");
        assert_eq!(payload["result"]["defaults"]["frequency"], "MONTHLY");
        assert_eq!(
            payload["result"]["defaults"]["payment_method"],
            "REFERENCE_ACCOUNT"
        );
        assert_eq!(payload["result"]["schedules"][0]["day_of_month"], 1);

        config_mock.assert();
    }

    #[test]
    fn run_broker_command_human_routes_savings_plan_config_to_text_output() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());

        let config_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_authorization_header())
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanConfig".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "account": {
                            "brokerPortfolio": {
                                "security": {
                                    "isin": "US0378331005",
                                    "name": "Apple Inc.",
                                    "type": "STOCK",
                                    "savingsPlanConfiguration": {
                                        "schedules": [
                                            {
                                                "dayOfTheMonth": 1,
                                                "isEarliest": true,
                                                "isDefault": true,
                                                "yearMonths": [
                                                    { "yearMonth": "2026-07", "isAvailable": true }
                                                ]
                                            }
                                        ],
                                        "minSavingsPlanAmount": "25",
                                        "maxSavingsPlanAmount": "5000",
                                        "defaultMinSavingsPlanAmount": "25",
                                        "dynamizationRates": [0],
                                        "defaultDynamizationRate": 0,
                                        "paymentMethods": ["REFERENCE_ACCOUNT"],
                                        "frequencies": ["MONTHLY"]
                                    }
                                }
                            }
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
                command: crate::cli::BrokerCommand::SavingsPlans(
                    crate::cli::BrokerSavingsPlansArgs {
                        command: Some(crate::cli::BrokerSavingsPlansCommand::Config(
                            crate::cli::BrokerSavingsPlanConfigArgs {
                                portfolio_id: Some("portfolio-1".to_string()),
                                isin: "US0378331005".to_string(),
                                json: false,
                            },
                        )),
                        portfolio_id: None,
                        json: false,
                    },
                ),
            },
            &config,
            &mut session_manager,
        )
        .expect("human config output");

        match output {
            HumanBrokerOutput::Text(lines) => {
                assert!(lines.iter().any(|line| line == "portfolio_id: portfolio-1"));
                assert!(lines.iter().any(|line| line == "security_name: Apple Inc."));
                assert!(
                    lines
                        .iter()
                        .any(|line| line == "default_frequency: MONTHLY")
                );
            }
            HumanBrokerOutput::Json(_, _) => panic!("expected text output for human config path"),
        }

        config_mock.assert();
    }

    #[test]
    fn execute_broker_savings_plan_config_maps_missing_configuration_to_unavailable_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());

        let config_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_authorization_header())
            .match_body(mockito::Matcher::Regex(
                "BrokerSavingsPlanConfig".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "account": {
                            "brokerPortfolio": {
                                "security": {
                                    "isin": "US0378331005",
                                    "name": "Apple Inc.",
                                    "type": "STOCK",
                                    "savingsPlanConfiguration": null
                                }
                            }
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

        let err = execute_broker_savings_plan_config(
            crate::cli::BrokerSavingsPlanConfigArgs {
                portfolio_id: Some("portfolio-1".to_string()),
                isin: "US0378331005".to_string(),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .unwrap_err();

        assert!(err.to_string().contains("SAVINGS_PLAN_CONFIG_UNAVAILABLE"));

        config_mock.assert();
    }

    #[test]
    fn render_broker_cash_breakdown_text_uses_public_field_names() {
        let payload = json!({
            "result": {
                "cash_balance": "10",
                "buying_power": "110",
                "buying_power_without_credit": "120",
                "available_credit_line": "20",
                "loaned": "30",
                "pending_buy_orders_amount": "40",
                "possible_taxes": "90",
                "derivatives_buying_power": "130",
                "available_for_derivatives": "160"
            }
        });

        let lines = render_broker_cash_breakdown_text(&payload);

        assert_eq!(
            lines,
            vec![
                "cash_balance: 10",
                "buying_power: 110",
                "buying_power_without_credit: 120",
                "available_credit_line: 20",
                "loaned: 30",
                "pending_buy_orders_amount: 40",
                "possible_taxes: 90",
                "derivatives_buying_power: 130",
                "available_for_derivatives: 160",
            ]
        );
    }

    #[test]
    fn render_broker_savings_plan_config_text_summarizes_defaults_and_allowed_values() {
        let payload = json!({
            "portfolio_id": "portfolio-1",
            "result": {
                "security": {
                    "isin": "US0378331005",
                    "name": "Apple Inc.",
                    "security_type": "STOCK"
                },
                "amount_limits": {
                    "min": "25",
                    "max": "5000"
                },
                "defaults": {
                    "frequency": "MONTHLY",
                    "day_of_month": 1,
                    "year_month": "2026-07",
                    "dynamization_rate": "0",
                    "payment_method": "REFERENCE_ACCOUNT"
                },
                "frequencies": ["MONTHLY", "QUARTERLY"],
                "payment_methods": ["REFERENCE_ACCOUNT", "CASH_BALANCE"],
                "dynamization_rates": ["0", "1.5"],
                "schedules": [{
                    "day_of_month": 1,
                    "is_default": true,
                    "is_earliest": true,
                    "available_year_months": ["2026-07", "2026-08"]
                }]
            }
        });

        let lines = render_broker_savings_plan_config_text(&payload);

        assert_eq!(lines[0], "portfolio_id: portfolio-1");
        assert!(
            lines
                .iter()
                .any(|line| line == "security_isin: US0378331005")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "default_frequency: MONTHLY")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "frequencies: MONTHLY, QUARTERLY")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "payment_methods: REFERENCE_ACCOUNT, CASH_BALANCE")
        );
        assert!(lines.iter().any(|line| {
            line == "schedule_1: day_of_month=1 default=true earliest=true available_year_months=2026-07, 2026-08"
        }));
    }

    #[test]
    fn render_broker_chart_text_summarizes_non_empty_series() {
        let first_mid_price = json!(185.01);
        let min_mid_price = json!(183.50);
        let max_mid_price = json!(188.00);
        let first_mid_price_line = format!(
            "first_mid_price: {}",
            display_money(Some(&first_mid_price), Some("EUR"))
        );
        let last_mid_price_line = format!(
            "last_mid_price: {}",
            display_money(Some(&max_mid_price), Some("EUR"))
        );
        let min_mid_price_line = format!(
            "min_mid_price: {}",
            display_money(Some(&min_mid_price), Some("EUR"))
        );
        let max_mid_price_line = format!(
            "max_mid_price: {}",
            display_money(Some(&max_mid_price), Some("EUR"))
        );
        let payload = json!({
            "isin": "US0378331005",
            "timeframe": "1m",
            "currency": "EUR",
            "source": "CONSOLIDATED",
            "point_count": 3,
            "data_points": [
                {
                    "mid_price": 188.00,
                    "timestamp_utc": "2026-05-25T09:00:00Z"
                },
                {
                    "mid_price": 185.01,
                    "timestamp_utc": "2026-05-23T09:00:00Z"
                },
                {
                    "mid_price": 183.50,
                    "timestamp_utc": "2026-05-24T09:00:00Z"
                }
            ]
        });

        let lines = render_broker_chart_text(&payload);

        assert!(lines.iter().any(|line| line == "isin: US0378331005"));
        assert!(lines.iter().any(|line| line == "timeframe: 1m"));
        assert!(lines.iter().any(|line| line == "source: CONSOLIDATED"));
        assert!(lines.iter().any(|line| line == "point_count: 3"));
        assert!(
            lines
                .iter()
                .any(|line| line == "range_start: 2026-05-23T09:00:00Z")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "range_end: 2026-05-25T09:00:00Z")
        );
        assert!(lines.iter().any(|line| line == &first_mid_price_line));
        assert!(lines.iter().any(|line| line == &last_mid_price_line));
        assert!(lines.iter().any(|line| line == &min_mid_price_line));
        assert!(lines.iter().any(|line| line == &max_mid_price_line));
    }

    #[test]
    fn render_broker_chart_text_uses_none_placeholders_for_empty_series() {
        let payload = json!({
            "isin": "US0378331005",
            "timeframe": "ytd",
            "currency": "EUR",
            "source": "CONSOLIDATED",
            "point_count": 0,
            "data_points": []
        });

        let lines = render_broker_chart_text(&payload);

        assert!(lines.iter().any(|line| line == "point_count: 0"));
        assert!(lines.iter().any(|line| line == "range_start: <none>"));
        assert!(lines.iter().any(|line| line == "range_end: <none>"));
        assert!(lines.iter().any(|line| line == "first_mid_price: <none>"));
        assert!(lines.iter().any(|line| line == "last_mid_price: <none>"));
        assert!(lines.iter().any(|line| line == "min_mid_price: <none>"));
        assert!(lines.iter().any(|line| line == "max_mid_price: <none>"));
    }

    #[test]
    fn render_broker_chart_text_supports_wrapped_result_payload() {
        let payload = json!({
            "result": {
                "isin": "US0378331005",
                "timeframe": "1m",
                "currency": "EUR",
                "source": "CONSOLIDATED",
                "point_count": 1,
                "data_points": [
                    {
                        "mid_price": 185.01,
                        "timestamp_utc": "2026-05-23T09:00:00Z"
                    }
                ]
            }
        });

        let lines = render_broker_chart_text(&payload);

        assert!(lines.iter().any(|line| line == "isin: US0378331005"));
        assert!(lines.iter().any(|line| line == "timeframe: 1m"));
        assert!(lines.iter().any(|line| line == "source: CONSOLIDATED"));
        assert!(lines.iter().any(|line| line == "point_count: 1"));
    }

    #[test]
    fn render_broker_chart_text_uses_projected_point_count() {
        let payload = json!({
            "isin": "US0378331005",
            "timeframe": "1m",
            "currency": "EUR",
            "source": "CONSOLIDATED",
            "point_count": 9,
            "data_points": [
                {
                    "mid_price": 185.01,
                    "timestamp_utc": "2026-05-23T09:00:00Z"
                }
            ]
        });

        let lines = render_broker_chart_text(&payload);

        assert!(lines.iter().any(|line| line == "point_count: 9"));
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
