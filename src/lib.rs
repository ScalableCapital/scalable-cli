#![warn(clippy::all)]

mod auth;
mod broker_commands;
mod broker_context;
mod broker_projections;
mod broker_queries;
mod broker_query_execution;
mod broker_shared;
mod channel;
mod cli;
mod command_handlers;
mod config;
pub mod dpop;
mod graphql;
mod helpers;
mod installation_code;
mod machine;
pub mod session;
pub mod token;
mod token_verifier;
pub mod trade;
mod trade_attempt;
mod trade_confirmation;
mod trade_execution;
mod trade_presentation;
pub mod transport_security;
pub use crate::machine::{human_error_message, user_error_message};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::{Value, json};

use crate::auth::{
    REFRESH_RELOGIN_REQUIRED_PREFIX, load_authenticated_session_dpop_context,
    login_with_device_code, refresh_session_if_needed_with_dpop, refresh_session_with_dpop,
    revoke_tokens_on_logout_best_effort,
};
use crate::broker_commands::{
    HumanBrokerOutput, bootstrap_broker_context_after_login, run_broker_command_human,
    run_broker_command_machine,
};
use crate::broker_context::delete_context as delete_broker_context;
use crate::cli::{
    BrokerCommand, BrokerContextCommand, BrokerPriceAlertsCommand, BrokerSavingsPlansCommand,
    BrokerTradeCommand, BrokerTransactionCommand, BrokerWatchlistCommand, Cli, Commands,
    InstallationCodeArgs,
};
use crate::command_handlers::{run_human_whoami_command, run_machine_whoami_command};
use crate::config::{AppConfig, EnvConfig, TargetEnv};
use crate::dpop::DpopRuntimeOptions;
use crate::installation_code::load_or_create_installation_code;
use crate::machine::{print_error, print_success};
use crate::session::{Session, SessionManager, StoredSession};
use crate::trade::TradeSide;
use crate::trade_attempt::delete_attempt_store;
use crate::trade_confirmation::delete_confirmation_store;
use crate::trade_presentation::{
    presentation_required_leaf_paths as trade_presentation_required_leaf_paths,
    presentation_section_order_keys as trade_presentation_section_order_keys,
};
use crate::transport_security::validate_env_transport_security;

pub fn run() -> Result<()> {
    let Cli { command } = Cli::parse();
    let raw_machine_output = raw_command_requests_json_envelope(&command);
    let raw_command_name = machine_command_name(&command);
    let command = match normalize_command(command) {
        Ok(command) => command,
        Err(err) if raw_machine_output => {
            let exit_code = print_error(raw_command_name, &err);
            std::process::exit(exit_code);
        }
        Err(err) => return Err(err),
    };

    if let Commands::InstallationCode(args) = command {
        return run_installation_code_command(args, raw_command_name, raw_machine_output);
    }

    let config = AppConfig::load_or_default()?;
    let mut session_manager = SessionManager::new(&config)?;

    if command_requests_json_envelope(&command) {
        let command_name = machine_command_name(&command);
        match run_machine_command(command, &config, &mut session_manager) {
            Ok(data) => {
                print_success(command_name, data);
                return Ok(());
            }
            Err(err) => {
                let exit_code = print_error(command_name, &err);
                std::process::exit(exit_code);
            }
        }
    }

    run_human_command(command, &config, &mut session_manager)
}

fn command_requests_json_envelope(command: &Commands) -> bool {
    match command {
        Commands::InstallationCode(args) => args.json,
        Commands::Login(_) => false,
        Commands::Logout(args) => args.json,
        Commands::Whoami(args) => args.json,
        Commands::Broker(args) => broker_command_requests_json(&args.command),
        Commands::Capabilities(args) => args.json,
    }
}

fn raw_command_requests_json_envelope(command: &Commands) -> bool {
    match command {
        Commands::InstallationCode(args) => args.json,
        Commands::Broker(args) => raw_broker_command_requests_json(&args.command),
        other => command_requests_json_envelope(other),
    }
}

fn raw_broker_command_requests_json(command: &BrokerCommand) -> bool {
    match command {
        BrokerCommand::Watchlist(args) => match &args.command {
            Some(BrokerWatchlistCommand::Add(add_args)) => args.json || add_args.json,
            Some(BrokerWatchlistCommand::Remove(remove_args)) => args.json || remove_args.json,
            None => args.json,
        },
        BrokerCommand::PriceAlerts(args) => match &args.command {
            Some(BrokerPriceAlertsCommand::Add(add_args)) => args.json || add_args.json,
            Some(BrokerPriceAlertsCommand::Remove(remove_args)) => args.json || remove_args.json,
            None => args.json,
        },
        BrokerCommand::SavingsPlans(args) => match &args.command {
            Some(BrokerSavingsPlansCommand::Add(add_args)) => args.json || add_args.json,
            Some(BrokerSavingsPlansCommand::Remove(remove_args)) => args.json || remove_args.json,
            None => args.json,
        },
        other => broker_command_requests_json(other),
    }
}

fn normalize_command(command: Commands) -> Result<Commands> {
    match command {
        Commands::Broker(args) => Ok(Commands::Broker(crate::cli::BrokerArgs {
            command: normalize_broker_command(args.command)?,
        })),
        other => Ok(other),
    }
}

fn normalize_broker_command(command: BrokerCommand) -> Result<BrokerCommand> {
    match command {
        BrokerCommand::Watchlist(args) => {
            Ok(BrokerCommand::Watchlist(normalize_watchlist_args(args)?))
        }
        BrokerCommand::PriceAlerts(args) => Ok(BrokerCommand::PriceAlerts(
            normalize_price_alert_args(args)?,
        )),
        BrokerCommand::SavingsPlans(args) => Ok(BrokerCommand::SavingsPlans(
            normalize_savings_plan_args(args)?,
        )),
        other => Ok(other),
    }
}

fn normalize_watchlist_args(
    args: crate::cli::BrokerWatchlistArgs,
) -> Result<crate::cli::BrokerWatchlistArgs> {
    let crate::cli::BrokerWatchlistArgs {
        command,
        portfolio_id,
        include_year_to_date,
        quote_source,
        json,
    } = args;

    match command {
        Some(BrokerWatchlistCommand::Add(mut add_args)) => {
            reject_watchlist_list_only_flags(include_year_to_date, quote_source.as_deref())?;
            if add_args.portfolio_id.is_none() {
                add_args.portfolio_id = portfolio_id;
            }
            add_args.json |= json;
            Ok(crate::cli::BrokerWatchlistArgs {
                command: Some(BrokerWatchlistCommand::Add(add_args)),
                portfolio_id: None,
                include_year_to_date: false,
                quote_source: None,
                json: false,
            })
        }
        Some(BrokerWatchlistCommand::Remove(mut remove_args)) => {
            reject_watchlist_list_only_flags(include_year_to_date, quote_source.as_deref())?;
            if remove_args.portfolio_id.is_none() {
                remove_args.portfolio_id = portfolio_id;
            }
            remove_args.json |= json;
            Ok(crate::cli::BrokerWatchlistArgs {
                command: Some(BrokerWatchlistCommand::Remove(remove_args)),
                portfolio_id: None,
                include_year_to_date: false,
                quote_source: None,
                json: false,
            })
        }
        None => Ok(crate::cli::BrokerWatchlistArgs {
            command: None,
            portfolio_id,
            include_year_to_date,
            quote_source,
            json,
        }),
    }
}

fn reject_watchlist_list_only_flags(
    include_year_to_date: bool,
    quote_source: Option<&str>,
) -> Result<()> {
    if include_year_to_date {
        bail!(
            "Broker input invalid: `--include-year-to-date` is only supported for `sc broker watchlist` without `add` or `remove`"
        );
    }

    if quote_source.is_some() {
        bail!(
            "Broker input invalid: `--quote-source` is only supported for `sc broker watchlist` without `add` or `remove`"
        );
    }

    Ok(())
}

fn normalize_price_alert_args(
    args: crate::cli::BrokerPriceAlertsArgs,
) -> Result<crate::cli::BrokerPriceAlertsArgs> {
    let crate::cli::BrokerPriceAlertsArgs {
        command,
        portfolio_id,
        active_only,
        json,
    } = args;

    match command {
        Some(BrokerPriceAlertsCommand::Add(mut add_args)) => {
            reject_price_alert_list_only_flags(active_only)?;
            inherit_portfolio_id_and_json(
                &mut add_args.portfolio_id,
                &mut add_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerPriceAlertsArgs {
                command: Some(BrokerPriceAlertsCommand::Add(add_args)),
                portfolio_id: None,
                active_only: false,
                json: false,
            })
        }
        Some(BrokerPriceAlertsCommand::Remove(mut remove_args)) => {
            reject_price_alert_list_only_flags(active_only)?;
            inherit_portfolio_id_and_json(
                &mut remove_args.portfolio_id,
                &mut remove_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerPriceAlertsArgs {
                command: Some(BrokerPriceAlertsCommand::Remove(remove_args)),
                portfolio_id: None,
                active_only: false,
                json: false,
            })
        }
        None => Ok(crate::cli::BrokerPriceAlertsArgs {
            command: None,
            portfolio_id,
            active_only,
            json,
        }),
    }
}

fn reject_price_alert_list_only_flags(active_only: bool) -> Result<()> {
    if active_only {
        bail!(
            "Broker input invalid: `--active-only` is only supported for `sc broker price-alerts` without `add` or `remove`"
        );
    }

    Ok(())
}

fn normalize_savings_plan_args(
    args: crate::cli::BrokerSavingsPlansArgs,
) -> Result<crate::cli::BrokerSavingsPlansArgs> {
    let crate::cli::BrokerSavingsPlansArgs {
        command,
        portfolio_id,
        json,
    } = args;

    match command {
        Some(BrokerSavingsPlansCommand::Add(mut add_args)) => {
            inherit_portfolio_id_and_json(
                &mut add_args.portfolio_id,
                &mut add_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerSavingsPlansArgs {
                command: Some(BrokerSavingsPlansCommand::Add(add_args)),
                portfolio_id: None,
                json: false,
            })
        }
        Some(BrokerSavingsPlansCommand::Remove(mut remove_args)) => {
            inherit_portfolio_id_and_json(
                &mut remove_args.portfolio_id,
                &mut remove_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerSavingsPlansArgs {
                command: Some(BrokerSavingsPlansCommand::Remove(remove_args)),
                portfolio_id: None,
                json: false,
            })
        }
        None => Ok(crate::cli::BrokerSavingsPlansArgs {
            command: None,
            portfolio_id,
            json,
        }),
    }
}

fn inherit_portfolio_id_and_json(
    child_portfolio_id: &mut Option<String>,
    child_json: &mut bool,
    parent_portfolio_id: Option<String>,
    parent_json: bool,
) {
    if child_portfolio_id.is_none() {
        *child_portfolio_id = parent_portfolio_id;
    }
    *child_json |= parent_json;
}

fn broker_command_requests_json(command: &BrokerCommand) -> bool {
    match command {
        BrokerCommand::Context(context) => match &context.command {
            BrokerContextCommand::Show(args) => args.json,
            BrokerContextCommand::Select(args) => args.json,
        },
        BrokerCommand::Overview(args) => args.json,
        BrokerCommand::Analytics(args) => args.json,
        BrokerCommand::Transactions(args) => args.json,
        BrokerCommand::Transaction(transaction_args) => match &transaction_args.command {
            BrokerTransactionCommand::Details(args) => args.json,
        },
        BrokerCommand::Holdings(args) => args.json,
        BrokerCommand::Watchlist(args) => match &args.command {
            Some(BrokerWatchlistCommand::Add(add_args)) => add_args.json,
            Some(BrokerWatchlistCommand::Remove(remove_args)) => remove_args.json,
            None => args.json,
        },
        BrokerCommand::Search(args) => args.json,
        BrokerCommand::Quote(args) => args.json,
        BrokerCommand::SecurityNews(args) => args.json,
        BrokerCommand::PriceAlerts(args) => match &args.command {
            Some(BrokerPriceAlertsCommand::Add(add_args)) => add_args.json,
            Some(BrokerPriceAlertsCommand::Remove(remove_args)) => remove_args.json,
            None => args.json,
        },
        BrokerCommand::SavingsPlans(args) => match &args.command {
            Some(BrokerSavingsPlansCommand::Add(add_args)) => add_args.json,
            Some(BrokerSavingsPlansCommand::Remove(remove_args)) => remove_args.json,
            None => args.json,
        },
        BrokerCommand::Trade(trade_args) => match &trade_args.command {
            BrokerTradeCommand::Buy(args) => args.json,
            BrokerTradeCommand::Sell(args) => args.json,
            BrokerTradeCommand::Cancel(args) => args.json,
        },
    }
}

pub(crate) fn resolve_active_env(session_manager: &SessionManager) -> Result<TargetEnv> {
    let stored = session_manager.load_required_active()?;
    crate::channel::require_current_channel(stored.env)?;
    Ok(stored.env)
}

fn run_human_command(
    command: Commands,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<()> {
    match command {
        Commands::InstallationCode(_) => {
            unreachable!("installation-code is handled before config and session initialization")
        }
        Commands::Login(args) => {
            let _ = args;
            let env = crate::channel::current_env();
            let env_cfg = crate::channel::current_env_config();
            let dpop_options = crate::channel::current_dpop_runtime_options(config);
            validate_env_transport_security(&env_cfg)?;
            login_with_device_code(session_manager, env, &env_cfg, &dpop_options)?;
            finalize_login_human(session_manager, env, &env_cfg, &dpop_options, "device code")?;
        }
        Commands::Logout(_) => {
            cleanup_local_artifacts_on_logout_best_effort();
            if let Some(stored) = session_manager.load_active()? {
                if stored.env == crate::channel::current_env() {
                    let env_cfg = crate::channel::current_env_config();
                    let dpop_options = crate::channel::current_dpop_runtime_options(config);
                    revoke_tokens_on_logout_best_effort(&env_cfg, &stored.session, &dpop_options);
                }
                session_manager.delete_active()?;
                println!("Logged out.");
            } else {
                println!("No active session.");
            }
        }
        Commands::Whoami(args) => {
            run_human_whoami_command(args, config, session_manager)?;
        }
        Commands::Broker(args) => {
            let payload = run_broker_command_human(args, config, session_manager)?;
            match payload {
                HumanBrokerOutput::Json(value, compact) => {
                    if compact {
                        println!("{}", serde_json::to_string(&value)?);
                    } else {
                        println!("{}", serde_json::to_string_pretty(&value)?);
                    }
                }
                HumanBrokerOutput::Text(lines) => {
                    for line in lines {
                        println!("{line}");
                    }
                }
            }
        }
        Commands::Capabilities(_) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&machine_capabilities(config))?
            );
        }
    }

    Ok(())
}

fn run_machine_command(
    command: Commands,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    match command {
        Commands::InstallationCode(_) => {
            unreachable!("installation-code is handled before config and session initialization")
        }
        Commands::Login(_) => bail!("`login` does not support --json in this version"),
        Commands::Logout(_) => {
            cleanup_local_artifacts_on_logout_best_effort();
            if let Some(stored) = session_manager.load_active()? {
                if stored.env == crate::channel::current_env() {
                    let env_cfg = crate::channel::current_env_config();
                    let dpop_options = crate::channel::current_dpop_runtime_options(config);
                    revoke_tokens_on_logout_best_effort(&env_cfg, &stored.session, &dpop_options);
                }
                session_manager.delete_active()?;
                return Ok(json!({"logged_out": true}));
            }
            Ok(json!({"logged_out": false}))
        }
        Commands::Whoami(args) => run_machine_whoami_command(args, config, session_manager),
        Commands::Broker(args) => run_broker_command_machine(args, config, session_manager),
        Commands::Capabilities(_) => Ok(machine_capabilities(config)),
    }
}

fn cleanup_broker_context_best_effort() {
    let _ = delete_broker_context();
}

fn cleanup_local_artifacts_on_logout_best_effort() {
    cleanup_broker_context_best_effort();
    let _ = delete_attempt_store();
    let _ = delete_confirmation_store();
}

fn finalize_login_human(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    dpop_options: &DpopRuntimeOptions,
    mode_label: &str,
) -> Result<()> {
    println!("Logged in via {mode_label}.");
    cleanup_broker_context_best_effort();
    match bootstrap_broker_context_after_login(session_manager, env, env_cfg, dpop_options) {
        Ok(context) => {
            println!(
                "Broker context auto-saved: account_id={}, portfolio_id={}",
                context.account_id,
                context.portfolio_id.as_deref().unwrap_or("<unset>")
            );
        }
        Err(err) => {
            eprintln!("Warning: broker context bootstrap failed (non-blocking): {err:#}");
        }
    }
    Ok(())
}

fn machine_capabilities(_config: &AppConfig) -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "output": "json_envelope",
        "auth": {
            "modes": ["device"],
            "non_interactive_modes": []
        },
        "commands": [
            "installation-code",
            "login",
            "logout",
            "whoami",
            "broker.context.show",
            "broker.context.select",
            "broker.overview",
            "broker.analytics",
            "broker.transactions",
            "broker.transaction.details",
            "broker.holdings",
            "broker.watchlist",
            "broker.watchlist.add",
            "broker.watchlist.remove",
            "broker.search",
            "broker.quote",
            "broker.security-news",
            "broker.price-alerts",
            "broker.price-alerts.add",
            "broker.price-alerts.remove",
            "broker.savings-plans",
            "broker.savings-plans.add",
            "broker.savings-plans.remove",
            "broker.trade.buy",
            "broker.trade.sell",
            "broker.trade.cancel",
            "capabilities"
        ],
        "command_metadata": {
            "login": {
                "human_only": true,
                "json_supported": false
            }
        },
        "workflows": {
            "broker.trade.buy": {
                "mode": "two_phase_confirmation",
                "phase_1": "Run trade buy args without --confirm to preview and receive confirmation id.",
                "phase_2": "Repeat the same trade buy args with --confirm <id> to submit, and add --accept-unsuitable when phase 1 marks the instrument as not suitable.",
                "preferred_output": "json",
                "phase_1_command_template_json": "sc broker trade buy --isin <ISIN> --amount <AMOUNT> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --json",
                "phase_2_command_template_json": "sc broker trade buy --isin <ISIN> --amount <AMOUNT> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --confirm <CONFIRMATION_ID> [--accept-unsuitable] --json",
                "raw_json_not_recommended_for_humans": true,
                "phase_1_presentation_requirement": {
                    "rule_id": "pre_trade_full_disclosure_v1",
                    "must_present_all_information": true,
                    "instruction": "Before running phase 2, you MUST present all pre-trade information from phase 1 in a human-readable summary (not raw JSON), without omitting or changing any values. After presenting it, you MUST explicitly ask the user whether to proceed and receive an explicit affirmative confirmation from the user in a separate interaction step. You MUST NOT execute phase 2 automatically, implicitly, or in the same step as phase 1 output.",
                    "requires_explicit_user_confirmation_between_phases": true,
                    "forbid_automatic_phase_2_execution": true,
                    "confirmation_must_be_separate_step": true,
                    "format": "markdown_sections",
                    "section_order": trade_presentation_section_order_keys(TradeSide::Buy),
                    "required_leaf_paths": trade_presentation_required_leaf_paths(TradeSide::Buy),
                    "preserve_exact_values": true,
                    "display_null_as_literal": true,
                    "raw_json_only_on_user_request": true
                }
            },
            "broker.trade.sell": {
                "mode": "two_phase_confirmation",
                "phase_1": "Run trade sell args without --confirm to preview and receive confirmation id.",
                "phase_2": "Repeat the same trade sell args with --confirm <id> to submit, and add --accept-unsuitable when phase 1 marks the instrument as not suitable.",
                "preferred_output": "json",
                "phase_1_command_template_json": "sc broker trade sell --isin <ISIN> --shares <SHARES> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --json",
                "phase_2_command_template_json": "sc broker trade sell --isin <ISIN> --shares <SHARES> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --confirm <CONFIRMATION_ID> [--accept-unsuitable] --json",
                "raw_json_not_recommended_for_humans": true,
                "phase_1_presentation_requirement": {
                    "rule_id": "pre_trade_full_disclosure_v1",
                    "must_present_all_information": true,
                    "instruction": "Before running phase 2, you MUST present all pre-trade information from phase 1 in a human-readable summary (not raw JSON), without omitting or changing any values. After presenting it, you MUST explicitly ask the user whether to proceed and receive an explicit affirmative confirmation from the user in a separate interaction step. You MUST NOT execute phase 2 automatically, implicitly, or in the same step as phase 1 output.",
                    "requires_explicit_user_confirmation_between_phases": true,
                    "forbid_automatic_phase_2_execution": true,
                    "confirmation_must_be_separate_step": true,
                    "format": "markdown_sections",
                    "section_order": trade_presentation_section_order_keys(TradeSide::Sell),
                    "required_leaf_paths": trade_presentation_required_leaf_paths(TradeSide::Sell),
                    "preserve_exact_values": true,
                    "display_null_as_literal": true,
                    "raw_json_only_on_user_request": true
                }
            }
        },
        "exit_codes": {
            "validation_error": 10,
            "auth_or_config_error": 20,
            "network_or_backend_error": 30,
            "generic_error": 1
        }
    })
}

fn machine_command_name(command: &Commands) -> &'static str {
    match command {
        Commands::InstallationCode(_) => "installation-code",
        Commands::Login(_) => "login",
        Commands::Logout(_) => "logout",
        Commands::Whoami(_) => "whoami",
        Commands::Broker(broker) => match &broker.command {
            BrokerCommand::Context(context) => match &context.command {
                BrokerContextCommand::Show(_) => "broker.context.show",
                BrokerContextCommand::Select(_) => "broker.context.select",
            },
            BrokerCommand::Overview(_) => "broker.overview",
            BrokerCommand::Analytics(_) => "broker.analytics",
            BrokerCommand::Transactions(_) => "broker.transactions",
            BrokerCommand::Transaction(transaction_args) => match &transaction_args.command {
                BrokerTransactionCommand::Details(_) => "broker.transaction.details",
            },
            BrokerCommand::Holdings(_) => "broker.holdings",
            BrokerCommand::Watchlist(args) => match &args.command {
                Some(BrokerWatchlistCommand::Add(_)) => "broker.watchlist.add",
                Some(BrokerWatchlistCommand::Remove(_)) => "broker.watchlist.remove",
                None => "broker.watchlist",
            },
            BrokerCommand::Search(_) => "broker.search",
            BrokerCommand::Quote(_) => "broker.quote",
            BrokerCommand::SecurityNews(_) => "broker.security-news",
            BrokerCommand::PriceAlerts(args) => match &args.command {
                Some(BrokerPriceAlertsCommand::Add(_)) => "broker.price-alerts.add",
                Some(BrokerPriceAlertsCommand::Remove(_)) => "broker.price-alerts.remove",
                None => "broker.price-alerts",
            },
            BrokerCommand::SavingsPlans(args) => match &args.command {
                Some(BrokerSavingsPlansCommand::Add(_)) => "broker.savings-plans.add",
                Some(BrokerSavingsPlansCommand::Remove(_)) => "broker.savings-plans.remove",
                None => "broker.savings-plans",
            },
            BrokerCommand::Trade(trade_args) => match &trade_args.command {
                BrokerTradeCommand::Buy(_) => "broker.trade.buy",
                BrokerTradeCommand::Sell(_) => "broker.trade.sell",
                BrokerTradeCommand::Cancel(_) => "broker.trade.cancel",
            },
        },
        Commands::Capabilities(_) => "capabilities",
    }
}

fn run_installation_code_command(
    args: InstallationCodeArgs,
    raw_command_name: &'static str,
    raw_machine_output: bool,
) -> Result<()> {
    let value = match load_or_create_installation_code() {
        Ok(value) => value,
        Err(err) if raw_machine_output => {
            let exit_code = print_error(raw_command_name, &err);
            std::process::exit(exit_code);
        }
        Err(err) => return Err(err),
    };

    if args.json {
        print_success(
            raw_command_name,
            json!({
                "installation_code": value.installation_code,
                "display_code": value.display_code,
            }),
        );
    } else {
        println!("Installation code: {}", value.display_code);
        println!("Send this code to Scalable to request access to the allowlist.");
    }

    Ok(())
}

pub(crate) fn refresh_loaded_session_if_needed(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    stored_session: StoredSession,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Session> {
    let dpop = load_authenticated_session_dpop_context(&stored_session, dpop_options)?;
    if let Some(refreshed) =
        refresh_session_if_needed_with_dpop(env_cfg, &stored_session.session, &dpop)
            .map_err(|err| clear_active_session_on_refresh_relogin_failure(session_manager, err))?
    {
        save_active_session(session_manager, env, &refreshed, dpop.jwk_thumbprint())
            .map_err(|err| clear_active_session_on_refresh_relogin_failure(session_manager, err))?;
        return Ok(refreshed);
    }

    Ok(stored_session.session)
}

pub(crate) fn execute_with_refresh_retry<T, F>(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    session: &mut Session,
    dpop_options: &DpopRuntimeOptions,
    mut action: F,
) -> Result<T>
where
    F: FnMut(&str) -> Result<T>,
{
    match action(&session.access_token) {
        Ok(value) => Ok(value),
        Err(first_err)
            if is_unauthorized_graphql_error(&first_err) && session.refresh_token.is_some() =>
        {
            let dpop =
                load_validated_active_session_dpop_context(session_manager, env, dpop_options)?;
            let refreshed = refresh_session_with_dpop(env_cfg, session, &dpop)
                .context("Token refresh after unauthorized response failed")
                .map_err(|err| {
                    clear_active_session_on_refresh_relogin_failure(session_manager, err)
                })?;
            save_active_session(session_manager, env, &refreshed, dpop.jwk_thumbprint()).map_err(
                |err| clear_active_session_on_refresh_relogin_failure(session_manager, err),
            )?;
            *session = refreshed;

            action(&session.access_token)
                .context("GraphQL call failed after refreshing access token")
        }
        Err(first_err) => Err(first_err),
    }
}

fn is_unauthorized_graphql_error(err: &anyhow::Error) -> bool {
    let detail = err.to_string();
    if detail.contains("GraphQL HTTP error 401") {
        return true;
    }

    // Some GraphQL backends return auth failures in a 200 payload with errors[].
    if detail.contains("GraphQL returned errors")
        && (detail.contains("UNAUTHENTICATED")
            || detail.contains("\"code\":\"UNAUTHENTICATED\"")
            || detail.contains("Missing or invalid credentials"))
    {
        return true;
    }

    false
}

fn save_active_session(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    session: &Session,
    dpop_jwk_thumbprint: &str,
) -> Result<()> {
    session_manager
        .save_active(&StoredSession {
            env,
            session: session.clone(),
            dpop_jwk_thumbprint: Some(dpop_jwk_thumbprint.to_string()),
        })
        .map_err(|_err| {
            anyhow::anyhow!(
                "{REFRESH_RELOGIN_REQUIRED_PREFIX} Token refresh succeeded but the rotated session could not be persisted locally. Run 'sc login'."
            )
        })
}

fn load_validated_active_session_dpop_context(
    session_manager: &SessionManager,
    env: TargetEnv,
    dpop_options: &DpopRuntimeOptions,
) -> Result<auth::AuthDpopContext> {
    let stored = session_manager.load_required_active()?;
    if stored.env != env {
        bail!(
            "Stored session belongs to {}, not {env}. Run 'sc login' to replace it.",
            stored.env
        );
    }
    load_authenticated_session_dpop_context(&stored, dpop_options)
}

fn clear_active_session_on_refresh_relogin_failure(
    session_manager: &mut SessionManager,
    err: anyhow::Error,
) -> anyhow::Error {
    if !error_chain_contains_refresh_relogin_prefix(&err) {
        return err;
    }

    match session_manager.delete_active() {
        Ok(()) => err,
        Err(delete_err) => anyhow::anyhow!(
            "{REFRESH_RELOGIN_REQUIRED_PREFIX} {} Failed clearing the stale active session locally: {delete_err}",
            user_error_message(&err)
        ),
    }
}

fn error_chain_contains_refresh_relogin_prefix(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.to_string().contains(REFRESH_RELOGIN_REQUIRED_PREFIX))
}

pub(crate) fn print_whoami_text(result: &Value) -> Result<()> {
    let lines = build_whoami_lines(result)?;
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

fn build_whoami_lines(result: &Value) -> Result<Vec<String>> {
    let obj = result
        .get("personOverview")
        .and_then(Value::as_object)
        .context("personOverview is missing in response")?;

    let id = obj.get("id").and_then(Value::as_str).unwrap_or("<unknown>");
    let locale = obj
        .get("locale")
        .and_then(Value::as_str)
        .unwrap_or("<none>");

    let first_name = obj
        .get("personalDetails")
        .and_then(|v| v.get("firstName"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let last_name = obj
        .get("personalDetails")
        .and_then(|v| v.get("lastName"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let full_name = format!("{first_name} {last_name}");
    let full_name = full_name.trim();
    let display_name = if full_name.is_empty() {
        "<none>"
    } else {
        full_name
    };

    let lines = vec![
        format!("name: {display_name}"),
        format!("id: {id}"),
        format!("locale: {locale}"),
    ];

    Ok(lines)
}

#[cfg(test)]
fn test_env_mutex() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
pub(crate) fn lock_test_env() -> std::sync::MutexGuard<'static, ()> {
    match test_env_mutex().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::cli::{BrokerArgs, Cli, Commands, LogoutArgs};
    use crate::config::{
        AppConfig, AuthConfig, DpopKeyBackend, EnvConfig, RuntimeAuthConfig,
        SessionBackendPreference,
    };
    use crate::session::{FileStore, LoginSource, StorageBackend, StoredSession};
    use mockito::Server;
    use tempfile::tempdir;

    struct EnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl Into<OsString>) -> Self {
            let previous = std::env::var_os(key);
            let value = value.into();
            unsafe {
                std::env::set_var(key, &value);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn build_whoami_lines_renders_identity() {
        let result = serde_json::json!({
            "personOverview": {
                "id": "person-1",
                "locale": "de-DE",
                "personalDetails": {
                    "firstName": "Ada",
                    "lastName": "Lovelace"
                }
            }
        });
        let lines = build_whoami_lines(&result).expect("whoami lines should render");
        assert!(lines.contains(&"name: Ada Lovelace".to_string()));
    }

    #[test]
    fn unauthorized_detection_accepts_http_401_graphql_errors() {
        let err = anyhow::anyhow!("GraphQL HTTP error 401: unauthorized");
        assert!(is_unauthorized_graphql_error(&err));
    }

    #[test]
    fn unauthorized_detection_accepts_graphql_unauthenticated_payload_errors() {
        let err = anyhow::Error::msg("GraphQL returned errors for WhoAmI (code: UNAUTHENTICATED)");
        assert!(is_unauthorized_graphql_error(&err));
    }

    #[test]
    fn unauthorized_detection_ignores_non_auth_graphql_errors() {
        let err = anyhow::Error::msg("GraphQL returned errors for WhoAmI (code: FORBIDDEN)");
        assert!(!is_unauthorized_graphql_error(&err));
    }

    #[test]
    fn unauthorized_detection_ignores_rate_limited_errors() {
        let err = anyhow::Error::msg(
            "RATE_LIMITED: backend rate limit exceeded during BrokerOverview; retry after 30s",
        );
        assert!(!is_unauthorized_graphql_error(&err));
    }

    #[test]
    fn execute_with_refresh_retry_does_not_retry_rate_limited_errors() {
        let tmp = tempdir().expect("tempdir");
        let env_cfg = crate::channel::current_env_config();
        let config = AppConfig {
            auth: RuntimeAuthConfig {
                session_backend: SessionBackendPreference::File,
                signing_key_backend: DpopKeyBackend::File,
                pkcs11: None,
            },
        };
        let store =
            StorageBackend::File(FileStore::new(tmp.path().to_path_buf()).expect("file store"));
        let mut session_manager = SessionManager::with_store(store);
        let dpop_options = crate::channel::current_dpop_runtime_options(&config);
        let mut session = Session {
            access_token: "access-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        };
        let mut attempts = 0;

        let result: Result<()> = execute_with_refresh_retry(
            &mut session_manager,
            crate::channel::current_env(),
            &env_cfg,
            &mut session,
            &dpop_options,
            |_| {
                attempts += 1;
                Err(anyhow::anyhow!(
                    "RATE_LIMITED: backend rate limit exceeded during BrokerOverview; retry after 30s"
                ))
            },
        );
        let err = result.expect_err("rate-limited call should fail");

        assert_eq!(attempts, 1);
        assert!(err.to_string().contains("RATE_LIMITED:"));
        assert_eq!(session.access_token, "access-token");
        assert_eq!(session.refresh_token.as_deref(), Some("refresh-token"));
    }

    #[test]
    fn machine_logout_deletes_local_session_when_dpop_key_is_missing() {
        let _lock = crate::lock_test_env();
        let tmp = tempdir().expect("tempdir");
        let server = Server::new();
        let _config_dir = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(EnvConfig {
            graphql_url: format!("{}/graphql", server.url()),
            auth: AuthConfig {
                issuer: server.url(),
                audience: "aud".to_string(),
                client_id: "client-id".to_string(),
            },
        });

        let config = AppConfig {
            auth: RuntimeAuthConfig {
                session_backend: SessionBackendPreference::File,
                signing_key_backend: DpopKeyBackend::File,
                pkcs11: None,
            },
        };
        let store =
            StorageBackend::File(FileStore::new(tmp.path().to_path_buf()).expect("file store"));
        let mut session_manager = SessionManager::with_store(store);
        session_manager
            .save_active(&StoredSession {
                env: crate::channel::current_env(),
                session: Session {
                    access_token: "access-token".to_string(),
                    refresh_token: Some("refresh-token".to_string()),
                    id_token: None,
                    expires_at: Some(9_999_999_999),
                    person_id: "person-1".to_string(),
                    source: LoginSource::DeviceCode,
                },
                dpop_jwk_thumbprint: Some("thumbprint-1".to_string()),
            })
            .expect("save session");

        let result = run_machine_command(
            Commands::Logout(LogoutArgs { json: true }),
            &config,
            &mut session_manager,
        )
        .expect("logout should succeed");

        assert_eq!(result["logged_out"], serde_json::json!(true));
        assert!(
            session_manager
                .load_active()
                .expect("load active")
                .is_none()
        );
    }

    #[test]
    fn watchlist_add_inherits_parent_portfolio_and_json_flags() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "watchlist",
            "--portfolio-id",
            "p-parent",
            "--json",
            "add",
            "--isin",
            "US0378331005",
        ]);

        let normalized = normalize_command(command).expect("command should normalize");
        assert!(command_requests_json_envelope(&normalized));

        match normalized {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Watchlist(args),
            }) => match args.command {
                Some(BrokerWatchlistCommand::Add(add_args)) => {
                    assert_eq!(add_args.portfolio_id.as_deref(), Some("p-parent"));
                    assert!(add_args.json);
                }
                _ => panic!("expected add subcommand"),
            },
            _ => panic!("expected broker watchlist command"),
        }
    }

    #[test]
    fn watchlist_remove_inherits_parent_portfolio_and_json_flags() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "watchlist",
            "--portfolio-id",
            "p-parent",
            "--json",
            "remove",
            "--isin",
            "US0378331005",
        ]);

        let normalized = normalize_command(command).expect("command should normalize");
        assert!(command_requests_json_envelope(&normalized));

        match normalized {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::Watchlist(args),
            }) => match args.command {
                Some(BrokerWatchlistCommand::Remove(remove_args)) => {
                    assert_eq!(remove_args.portfolio_id.as_deref(), Some("p-parent"));
                    assert!(remove_args.json);
                }
                _ => panic!("expected remove subcommand"),
            },
            _ => panic!("expected broker watchlist command"),
        }
    }

    #[test]
    fn watchlist_remove_rejects_list_only_flags_on_mutation_path() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "watchlist",
            "--quote-source",
            "CONSOLIDATED",
            "remove",
            "--isin",
            "US0378331005",
        ]);

        let err = normalize_command(command).unwrap_err();
        assert!(err.to_string().contains("--quote-source"));
    }

    #[test]
    fn watchlist_add_rejects_blank_quote_source_on_mutation_path() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "watchlist",
            "--quote-source",
            "",
            "add",
            "--isin",
            "US0378331005",
        ]);

        let err = normalize_command(command).unwrap_err();
        assert!(err.to_string().contains("--quote-source"));
    }

    #[test]
    fn price_alert_add_inherits_parent_portfolio_and_json_flags() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "price-alerts",
            "--portfolio-id",
            "p-parent",
            "--json",
            "add",
            "--isin",
            "US0378331005",
            "--price",
            "123.45",
        ]);

        let normalized = normalize_command(command).expect("command should normalize");
        assert!(command_requests_json_envelope(&normalized));

        match normalized {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::PriceAlerts(args),
            }) => match args.command {
                Some(BrokerPriceAlertsCommand::Add(add_args)) => {
                    assert_eq!(add_args.portfolio_id.as_deref(), Some("p-parent"));
                    assert!(add_args.json);
                }
                _ => panic!("expected add subcommand"),
            },
            _ => panic!("expected broker price-alerts command"),
        }
    }

    #[test]
    fn price_alert_add_rejects_list_only_flags_on_mutation_path() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "price-alerts",
            "--active-only",
            "add",
            "--isin",
            "US0378331005",
            "--price",
            "123.45",
        ]);

        let err = normalize_command(command).unwrap_err();
        assert!(err.to_string().contains("--active-only"));
    }

    #[test]
    fn price_alert_remove_inherits_parent_portfolio_and_json_flags() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "price-alerts",
            "--portfolio-id",
            "p-parent",
            "--json",
            "remove",
            "--alert-id",
            "alert-1",
        ]);

        let normalized = normalize_command(command).expect("command should normalize");
        assert!(command_requests_json_envelope(&normalized));

        match normalized {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::PriceAlerts(args),
            }) => match args.command {
                Some(BrokerPriceAlertsCommand::Remove(remove_args)) => {
                    assert_eq!(remove_args.portfolio_id.as_deref(), Some("p-parent"));
                    assert!(remove_args.json);
                }
                _ => panic!("expected remove subcommand"),
            },
            _ => panic!("expected broker price-alerts command"),
        }
    }

    #[test]
    fn price_alert_remove_rejects_list_only_flags_on_mutation_path() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "price-alerts",
            "--active-only",
            "remove",
            "--alert-id",
            "alert-1",
        ]);

        let err = normalize_command(command).unwrap_err();
        assert!(err.to_string().contains("--active-only"));
    }

    #[test]
    fn savings_plan_add_inherits_parent_portfolio_and_json_flags() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "savings-plans",
            "--portfolio-id",
            "p-parent",
            "--json",
            "add",
            "--isin",
            "US0378331005",
            "--amount",
            "100",
        ]);

        let normalized = normalize_command(command).expect("command should normalize");
        assert!(command_requests_json_envelope(&normalized));

        match normalized {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::SavingsPlans(args),
            }) => match args.command {
                Some(BrokerSavingsPlansCommand::Add(add_args)) => {
                    assert_eq!(add_args.portfolio_id.as_deref(), Some("p-parent"));
                    assert!(add_args.json);
                }
                _ => panic!("expected add subcommand"),
            },
            _ => panic!("expected broker savings-plans command"),
        }
    }

    #[test]
    fn savings_plan_remove_inherits_parent_portfolio_and_json_flags() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "savings-plans",
            "--portfolio-id",
            "p-parent",
            "--json",
            "remove",
            "--isin",
            "US0378331005",
        ]);

        let normalized = normalize_command(command).expect("command should normalize");
        assert!(command_requests_json_envelope(&normalized));

        match normalized {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::SavingsPlans(args),
            }) => match args.command {
                Some(BrokerSavingsPlansCommand::Remove(remove_args)) => {
                    assert_eq!(remove_args.portfolio_id.as_deref(), Some("p-parent"));
                    assert!(remove_args.json);
                }
                _ => panic!("expected remove subcommand"),
            },
            _ => panic!("expected broker savings-plans command"),
        }
    }
}
