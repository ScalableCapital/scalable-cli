#![warn(clippy::all)]

mod active_session;
mod auth;
mod broker_commands;
mod broker_context;
mod broker_portfolio_groups;
mod broker_portfolio_groups_projections;
mod broker_portfolio_groups_queries;
mod broker_portfolio_groups_render;
mod broker_projections;
mod broker_queries;
mod broker_query_execution;
mod broker_shared;
mod channel;
mod cli;
mod command_handlers;
mod config;
pub mod dpop;
mod execution_context;
mod fixed_term_commands;
mod fixed_term_projections;
mod fixed_term_queries;
#[cfg(test)]
mod fixed_term_tests;
mod graphql;
mod helpers;
mod machine;
mod overnight_commands;
mod overnight_projections;
mod overnight_queries;
mod overnight_query_execution;
mod overnight_shared;
mod payload_fingerprint;
mod savings_plan_confirmation;
mod savings_plan_presentation;
pub mod session;
mod session_refresh;
pub mod token;
mod token_verifier;
pub mod trade;
mod trade_attempt;
mod trade_confirmation;
mod trade_controls;
mod trade_execution;
mod trade_presentation;
pub mod transport_security;
pub use crate::machine::{human_error_message, user_error_message};

use anyhow::{Context, Result, bail};
use clap::{Parser, error::ErrorKind};
use serde_json::{Value, json};

use crate::auth::{login_with_device_code, revoke_tokens_on_logout_best_effort};
use crate::broker_commands::{
    HumanBrokerOutput, bootstrap_broker_context_after_login, run_broker_command_human,
    run_broker_command_machine,
};
use crate::broker_context::delete_context as delete_broker_context;
use crate::cli::{
    BrokerCommand, BrokerContextCommand, BrokerDerivativesCommand, BrokerPortfolioGroupsCommand,
    BrokerPriceAlertsCommand, BrokerSavingsPlansCommand, BrokerTradeCommand,
    BrokerTransactionCommand, BrokerWatchlistCommand, Cli, Commands, FixedTermCommand,
    OvernightCommand,
};
use crate::command_handlers::{run_human_whoami_command, run_machine_whoami_command};
use crate::config::{AppConfig, EnvConfig, TargetEnv};
use crate::dpop::DpopRuntimeOptions;
use crate::execution_context::{ExecutionContext, ParseFailureContext};
use crate::machine::{print_clap_error, print_error, print_success};
use crate::overnight_commands::{
    HumanOvernightOutput, run_overnight_command_human, run_overnight_command_machine,
};
use crate::savings_plan_confirmation::clear_on_logout as clear_savings_plan_confirmation_on_logout;
use crate::savings_plan_presentation::{
    COMPLIANCE_RULE_ID as SAVINGS_PLAN_COMPLIANCE_RULE_ID,
    PRESENTATION_FORMAT as SAVINGS_PLAN_PRESENTATION_FORMAT,
    required_leaf_paths as savings_plan_required_leaf_paths,
};
use crate::session::{SessionManager, SessionMode};
use crate::trade::TradeSide;
use crate::trade_attempt::delete_attempt_store;
use crate::trade_confirmation::delete_confirmation_store;
use crate::trade_controls::TradeControlsPolicy;
use crate::trade_presentation::{
    presentation_required_leaf_paths as trade_presentation_required_leaf_paths,
    presentation_section_order_keys as trade_presentation_section_order_keys,
};
use crate::transport_security::validate_env_transport_security;

pub fn run() -> Result<()> {
    let raw_args: Vec<_> = std::env::args_os().collect();
    let Cli { command } = match Cli::try_parse_from(raw_args.clone()) {
        Ok(cli) => cli,
        Err(err) => {
            let context = ParseFailureContext::from_raw_args_and_error(&raw_args, &err);
            match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => err.exit(),
                _ if context.requests_machine_output() => {
                    let exit_code = print_clap_error(&context.command_name, &err);
                    std::process::exit(exit_code);
                }
                _ => err.exit(),
            }
        }
    };
    let pre_normalization_context =
        ExecutionContext::from_raw_args_and_command(&raw_args, &command);
    let command = match normalize_command(command) {
        Ok(command) => command,
        Err(err) if pre_normalization_context.requests_machine_output() => {
            let exit_code = print_error(pre_normalization_context.command_name, &err);
            std::process::exit(exit_code);
        }
        Err(err) => return Err(err),
    };
    let execution_context = ExecutionContext::from_command(&command);

    let config = AppConfig::load_or_default()?;
    let mut session_manager = SessionManager::new(&config)?;

    if execution_context.requests_machine_output() {
        match run_machine_command(command, &config, &mut session_manager) {
            Ok(data) => {
                print_success(execution_context.command_name, data);
                return Ok(());
            }
            Err(err) => {
                let exit_code = print_error(execution_context.command_name, &err);
                std::process::exit(exit_code);
            }
        }
    }

    run_human_command(command, &config, &mut session_manager)
}

pub(crate) fn command_requests_json_envelope(command: &Commands) -> bool {
    match command {
        Commands::Login(_) => false,
        Commands::Logout(args) => args.json,
        Commands::Whoami(args) => args.json,
        Commands::FixedTerm(args) => match &args.command {
            FixedTermCommand::List(args) => args.json,
            FixedTermCommand::Details(args) => args.json,
        },
        Commands::Overnight(args) => match &args.command {
            Some(OvernightCommand::Transactions(transactions_args)) => transactions_args.json,
            None => args.json,
        },
        Commands::Broker(args) => broker_command_requests_json(&args.command),
        Commands::Capabilities(args) => args.json,
    }
}

fn normalize_command(command: Commands) -> Result<Commands> {
    match command {
        Commands::Overnight(args) => Ok(Commands::Overnight(normalize_overnight_args(args)?)),
        Commands::Broker(args) => Ok(Commands::Broker(crate::cli::BrokerArgs {
            command: normalize_broker_command(args.command)?,
        })),
        other => Ok(other),
    }
}

fn normalize_overnight_args(args: crate::cli::OvernightArgs) -> Result<crate::cli::OvernightArgs> {
    let crate::cli::OvernightArgs {
        command,
        savings_account_id,
        json,
    } = args;

    match command {
        Some(OvernightCommand::Transactions(mut transactions_args)) => {
            if transactions_args.savings_account_id.is_none() {
                transactions_args.savings_account_id = savings_account_id;
            }
            transactions_args.json |= json;
            Ok(crate::cli::OvernightArgs {
                command: Some(OvernightCommand::Transactions(transactions_args)),
                savings_account_id: None,
                json: false,
            })
        }
        None => Ok(crate::cli::OvernightArgs {
            command: None,
            savings_account_id,
            json,
        }),
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
        BrokerCommand::PortfolioGroups(args) => Ok(BrokerCommand::PortfolioGroups(
            normalize_portfolio_groups_args(args)?,
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
        Some(BrokerSavingsPlansCommand::Config(mut config_args)) => {
            inherit_portfolio_id_and_json(
                &mut config_args.portfolio_id,
                &mut config_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerSavingsPlansArgs {
                command: Some(BrokerSavingsPlansCommand::Config(config_args)),
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

fn normalize_portfolio_groups_args(
    args: crate::cli::BrokerPortfolioGroupsArgs,
) -> Result<crate::cli::BrokerPortfolioGroupsArgs> {
    let crate::cli::BrokerPortfolioGroupsArgs {
        command,
        portfolio_id,
        group_id,
        json,
    } = args;

    match command {
        Some(BrokerPortfolioGroupsCommand::Create(mut create_args)) => {
            reject_portfolio_groups_read_only_group_id(group_id.as_deref())?;
            inherit_portfolio_id_and_json(
                &mut create_args.portfolio_id,
                &mut create_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerPortfolioGroupsArgs {
                command: Some(BrokerPortfolioGroupsCommand::Create(create_args)),
                portfolio_id: None,
                group_id: None,
                json: false,
            })
        }
        Some(BrokerPortfolioGroupsCommand::Update(mut update_args)) => {
            reject_portfolio_groups_read_only_group_id(group_id.as_deref())?;
            if update_args.name.is_none()
                && update_args.description.is_none()
                && !update_args.clear_description
            {
                bail!(
                    "Broker input invalid: one of `--name`, `--description`, or `--clear-description` is required for `sc broker portfolio-groups update`"
                );
            }
            inherit_portfolio_id_and_json(
                &mut update_args.portfolio_id,
                &mut update_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerPortfolioGroupsArgs {
                command: Some(BrokerPortfolioGroupsCommand::Update(update_args)),
                portfolio_id: None,
                group_id: None,
                json: false,
            })
        }
        Some(BrokerPortfolioGroupsCommand::Delete(mut delete_args)) => {
            reject_portfolio_groups_read_only_group_id(group_id.as_deref())?;
            inherit_portfolio_id_and_json(
                &mut delete_args.portfolio_id,
                &mut delete_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerPortfolioGroupsArgs {
                command: Some(BrokerPortfolioGroupsCommand::Delete(delete_args)),
                portfolio_id: None,
                group_id: None,
                json: false,
            })
        }
        Some(BrokerPortfolioGroupsCommand::Assign(mut assign_args)) => {
            reject_portfolio_groups_read_only_group_id(group_id.as_deref())?;
            inherit_portfolio_id_and_json(
                &mut assign_args.portfolio_id,
                &mut assign_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerPortfolioGroupsArgs {
                command: Some(BrokerPortfolioGroupsCommand::Assign(assign_args)),
                portfolio_id: None,
                group_id: None,
                json: false,
            })
        }
        Some(BrokerPortfolioGroupsCommand::Unassign(mut unassign_args)) => {
            reject_portfolio_groups_read_only_group_id(group_id.as_deref())?;
            inherit_portfolio_id_and_json(
                &mut unassign_args.portfolio_id,
                &mut unassign_args.json,
                portfolio_id,
                json,
            );
            Ok(crate::cli::BrokerPortfolioGroupsArgs {
                command: Some(BrokerPortfolioGroupsCommand::Unassign(unassign_args)),
                portfolio_id: None,
                group_id: None,
                json: false,
            })
        }
        None => Ok(crate::cli::BrokerPortfolioGroupsArgs {
            command: None,
            portfolio_id,
            group_id,
            json,
        }),
    }
}

fn reject_portfolio_groups_read_only_group_id(group_id: Option<&str>) -> Result<()> {
    if group_id.is_some() {
        bail!(
            "Broker input invalid: `--group-id` is only supported for `sc broker portfolio-groups` without a subcommand"
        );
    }

    Ok(())
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
        BrokerCommand::CashBreakdown(args) => args.json,
        BrokerCommand::Transactions(args) => args.json,
        BrokerCommand::Transaction(transaction_args) => match &transaction_args.command {
            BrokerTransactionCommand::Details(args) => args.json,
        },
        BrokerCommand::Holdings(args) => args.json,
        BrokerCommand::PortfolioGroups(args) => match &args.command {
            Some(BrokerPortfolioGroupsCommand::Create(create_args)) => create_args.json,
            Some(BrokerPortfolioGroupsCommand::Update(update_args)) => update_args.json,
            Some(BrokerPortfolioGroupsCommand::Delete(delete_args)) => delete_args.json,
            Some(BrokerPortfolioGroupsCommand::Assign(assign_args)) => assign_args.json,
            Some(BrokerPortfolioGroupsCommand::Unassign(unassign_args)) => unassign_args.json,
            None => args.json,
        },
        BrokerCommand::Watchlist(args) => match &args.command {
            Some(BrokerWatchlistCommand::Add(add_args)) => add_args.json,
            Some(BrokerWatchlistCommand::Remove(remove_args)) => remove_args.json,
            None => args.json,
        },
        BrokerCommand::Search(args) => args.json,
        BrokerCommand::Derivatives(args) => match &args.command {
            BrokerDerivativesCommand::Search(search_args) => search_args.json,
        },
        BrokerCommand::Chart(args) => args.json,
        BrokerCommand::Quote(args) => args.json,
        BrokerCommand::SecurityNews(args) => args.json,
        BrokerCommand::PriceAlerts(args) => match &args.command {
            Some(BrokerPriceAlertsCommand::Add(add_args)) => add_args.json,
            Some(BrokerPriceAlertsCommand::Remove(remove_args)) => remove_args.json,
            None => args.json,
        },
        BrokerCommand::SavingsPlans(args) => match &args.command {
            Some(BrokerSavingsPlansCommand::Add(add_args)) => add_args.json,
            Some(BrokerSavingsPlansCommand::Config(config_args)) => config_args.json,
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
        Commands::Login(args) => {
            let env = crate::channel::current_env();
            let env_cfg = crate::channel::current_env_config();
            let dpop_options = crate::channel::current_dpop_runtime_options(config);
            let session_mode = args.local_read_only.then_some(SessionMode::LocalReadOnly);
            validate_env_transport_security(&env_cfg)?;
            login_with_device_code(session_manager, env, &env_cfg, session_mode, &dpop_options)?;
            finalize_login_human(
                session_manager,
                env,
                &env_cfg,
                &dpop_options,
                "device code",
                session_mode,
            )?;
        }
        Commands::Logout(_) => {
            cleanup_local_artifacts_on_logout_best_effort();
            if let Some(stored) = session_manager.load_active()? {
                if stored.env == crate::channel::current_env() {
                    let env_cfg = crate::channel::current_env_config();
                    let dpop_options = crate::channel::current_dpop_runtime_options(config);
                    revoke_tokens_on_logout_best_effort(
                        &env_cfg,
                        &stored.session,
                        stored.mode,
                        &dpop_options,
                    );
                }
                session_manager.delete_active_locked()?;
                println!("Logged out.");
            } else {
                println!("No active session.");
            }
        }
        Commands::Whoami(args) => {
            run_human_whoami_command(args, config, session_manager)?;
        }
        Commands::FixedTerm(args) => {
            let payload = fixed_term_commands::execute(args.command, config, session_manager)?;
            print_human_lines(&fixed_term_commands::render(&payload));
        }
        Commands::Overnight(args) => {
            let payload = run_overnight_command_human(args, config, session_manager)?;
            match payload {
                HumanOvernightOutput::Json(value, compact) => {
                    print_human_json(&value, compact)?;
                }
                HumanOvernightOutput::Text(lines) => print_human_lines(&lines),
            }
        }
        Commands::Broker(args) => {
            let payload = run_broker_command_human(args, config, session_manager)?;
            match payload {
                HumanBrokerOutput::Json(value, compact) => {
                    print_human_json(&value, compact)?;
                }
                HumanBrokerOutput::Text(lines) => print_human_lines(&lines),
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

fn print_human_json(value: &Value, compact: bool) -> Result<()> {
    let output = if compact {
        serde_json::to_string(value)?
    } else {
        serde_json::to_string_pretty(value)?
    };
    println!("{output}");
    Ok(())
}

fn print_human_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

fn run_machine_command(
    command: Commands,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    match command {
        Commands::Login(_) => bail!("`login` does not support --json in this version"),
        Commands::Logout(_) => {
            cleanup_local_artifacts_on_logout_best_effort();
            if let Some(stored) = session_manager.load_active()? {
                if stored.env == crate::channel::current_env() {
                    let env_cfg = crate::channel::current_env_config();
                    let dpop_options = crate::channel::current_dpop_runtime_options(config);
                    revoke_tokens_on_logout_best_effort(
                        &env_cfg,
                        &stored.session,
                        stored.mode,
                        &dpop_options,
                    );
                }
                session_manager.delete_active_locked()?;
                return Ok(json!({"logged_out": true}));
            }
            Ok(json!({"logged_out": false}))
        }
        Commands::Whoami(args) => run_machine_whoami_command(args, config, session_manager),
        Commands::FixedTerm(args) => {
            fixed_term_commands::execute(args.command, config, session_manager)
        }
        Commands::Overnight(args) => run_overnight_command_machine(args, config, session_manager),
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
    let _ = clear_savings_plan_confirmation_on_logout();
}

fn finalize_login_human(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    dpop_options: &DpopRuntimeOptions,
    mode_label: &str,
    session_mode: Option<SessionMode>,
) -> Result<()> {
    println!("Logged in via {mode_label}.");
    if session_mode == Some(SessionMode::LocalReadOnly) {
        println!("Local read-only mode is active for this session.");
    }
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

fn machine_capabilities(config: &AppConfig) -> Value {
    let trade_controls = TradeControlsPolicy::from_app_config(config).capabilities_payload();
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "output": "json_envelope",
        "auth": {
            "modes": ["device"],
            "non_interactive_modes": []
        },
        "commands": [
            "login",
            "logout",
            "whoami",
            "overnight",
            "overnight.transactions",
            "fixed-term.list",
            "fixed-term.details",
            "broker.context.show",
            "broker.context.select",
            "broker.overview",
            "broker.cash-breakdown",
            "broker.transactions",
            "broker.transaction.details",
            "broker.holdings",
            "broker.portfolio-groups",
            "broker.portfolio-groups.create",
            "broker.portfolio-groups.update",
            "broker.portfolio-groups.delete",
            "broker.portfolio-groups.assign",
            "broker.portfolio-groups.unassign",
            "broker.watchlist",
            "broker.watchlist.add",
            "broker.watchlist.remove",
            "broker.search",
            "broker.derivatives.search",
            "broker.chart",
            "broker.quote",
            "broker.security-news",
            "broker.price-alerts",
            "broker.price-alerts.add",
            "broker.price-alerts.remove",
            "broker.savings-plans",
            "broker.savings-plans.add",
            "broker.savings-plans.config",
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
                "phase_1": "Run trade buy args with exactly one of --amount or --shares, and without --confirm, to preview and receive confirmation id.",
                "phase_2": "Repeat the same trade buy args with exactly one of --amount or --shares and --confirm <id> to submit, and add --acknowledge-appropriateness-warning when phase 1 requires acknowledgement of an appropriateness warning.",
                "preferred_output": "json",
                "phase_1_command_template_json": "sc broker trade buy --isin <ISIN> --amount <AMOUNT> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --json",
                "phase_2_command_template_json": "sc broker trade buy --isin <ISIN> --amount <AMOUNT> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --confirm <CONFIRMATION_ID> [--acknowledge-appropriateness-warning] --json",
                "phase_1_command_template_json_shares": "sc broker trade buy --isin <ISIN> --shares <SHARES> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --json",
                "phase_2_command_template_json_shares": "sc broker trade buy --isin <ISIN> --shares <SHARES> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --confirm <CONFIRMATION_ID> [--acknowledge-appropriateness-warning] --json",
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
                "phase_2": "Repeat the same trade sell args with --confirm <id> to submit.",
                "preferred_output": "json",
                "phase_1_command_template_json": "sc broker trade sell --isin <ISIN> --shares <SHARES> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --json",
                "phase_2_command_template_json": "sc broker trade sell --isin <ISIN> --shares <SHARES> --order-type <market|limit|stop> [--limit-price <LIMIT_PRICE>] [--stop-price <STOP_PRICE>] [--venue <VENUE>] --confirm <CONFIRMATION_ID> --json",
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
            },
            "broker.savings-plans.add": {
                "mode": "two_phase_confirmation",
                "phase_1": "Run savings-plans add without --confirm to preview the full ex-ante cost disclosure and receive a confirmation id.",
                "phase_2": "Repeat the identical savings-plans add arguments with --confirm <id> only after explicit affirmative client confirmation in a separate interaction.",
                "preferred_output": "json",
                "phase_1_command_template_json": "sc broker savings-plans add --isin <ISIN> --amount <AMOUNT> [--frequency <FREQUENCY>] [--day-of-month <DAY>] [--year-month <YYYY-MM>] [--dynamization-rate <RATE>] [--payment-method <METHOD>] [--appropriateness-id <ID>] [--acknowledged-appropriateness-warning-version <VERSION>] --json",
                "phase_2_command_template_json": "sc broker savings-plans add --isin <ISIN> --amount <AMOUNT> [same options] --confirm <CONFIRMATION_ID> --json",
                "raw_json_not_recommended_for_humans": true,
                "phase_1_presentation_requirement": {
                    "rule_id": SAVINGS_PLAN_COMPLIANCE_RULE_ID,
                    "must_present_all_information": true,
                    "instruction": "Before running phase 2, you MUST present all phase-1 savings-plan information in a human-readable summary without omitting or changing values. Then obtain an explicit affirmative confirmation in a separate interaction. You MUST NOT execute phase 2 automatically, implicitly, or in the same step as phase 1 output.",
                    "requires_explicit_user_confirmation_between_phases": true,
                    "forbid_automatic_phase_2_execution": true,
                    "confirmation_must_be_separate_step": true,
                    "format": SAVINGS_PLAN_PRESENTATION_FORMAT,
                    "section_order": ["savings_plan", "ex_ante_costs", "confirmation"],
                    "required_leaf_paths": savings_plan_required_leaf_paths(),
                    "preserve_exact_values": true,
                    "display_null_as_literal": true,
                    "raw_json_only_on_user_request": true
                }
            }
        },
        "local_trade_controls": trade_controls,
        "exit_codes": {
            "validation_error": 10,
            "auth_or_config_error": 20,
            "network_or_backend_error": 30,
            "generic_error": 1
        }
    })
}

pub(crate) fn machine_command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Login(_) => "login",
        Commands::Logout(_) => "logout",
        Commands::Whoami(_) => "whoami",
        Commands::FixedTerm(args) => match &args.command {
            FixedTermCommand::List(_) => "fixed-term.list",
            FixedTermCommand::Details(_) => "fixed-term.details",
        },
        Commands::Overnight(args) => match &args.command {
            Some(OvernightCommand::Transactions(_)) => "overnight.transactions",
            None => "overnight",
        },
        Commands::Broker(broker) => broker_machine_command_name(&broker.command),
        Commands::Capabilities(_) => "capabilities",
    }
}

fn broker_machine_command_name(command: &BrokerCommand) -> &'static str {
    match command {
        BrokerCommand::Context(context) => match &context.command {
            BrokerContextCommand::Show(_) => "broker.context.show",
            BrokerContextCommand::Select(_) => "broker.context.select",
        },
        BrokerCommand::Overview(_) => "broker.overview",
        BrokerCommand::CashBreakdown(_) => "broker.cash-breakdown",
        BrokerCommand::Transactions(_) => "broker.transactions",
        BrokerCommand::Transaction(transaction_args) => match &transaction_args.command {
            BrokerTransactionCommand::Details(_) => "broker.transaction.details",
        },
        BrokerCommand::Holdings(_) => "broker.holdings",
        BrokerCommand::PortfolioGroups(args) => match &args.command {
            Some(BrokerPortfolioGroupsCommand::Create(_)) => "broker.portfolio-groups.create",
            Some(BrokerPortfolioGroupsCommand::Update(_)) => "broker.portfolio-groups.update",
            Some(BrokerPortfolioGroupsCommand::Delete(_)) => "broker.portfolio-groups.delete",
            Some(BrokerPortfolioGroupsCommand::Assign(_)) => "broker.portfolio-groups.assign",
            Some(BrokerPortfolioGroupsCommand::Unassign(_)) => "broker.portfolio-groups.unassign",
            None => "broker.portfolio-groups",
        },
        BrokerCommand::Watchlist(args) => match &args.command {
            Some(BrokerWatchlistCommand::Add(_)) => "broker.watchlist.add",
            Some(BrokerWatchlistCommand::Remove(_)) => "broker.watchlist.remove",
            None => "broker.watchlist",
        },
        BrokerCommand::Search(_) => "broker.search",
        BrokerCommand::Derivatives(args) => match &args.command {
            BrokerDerivativesCommand::Search(_) => "broker.derivatives.search",
        },
        BrokerCommand::Chart(_) => "broker.chart",
        BrokerCommand::Quote(_) => "broker.quote",
        BrokerCommand::SecurityNews(_) => "broker.security-news",
        BrokerCommand::PriceAlerts(args) => match &args.command {
            Some(BrokerPriceAlertsCommand::Add(_)) => "broker.price-alerts.add",
            Some(BrokerPriceAlertsCommand::Remove(_)) => "broker.price-alerts.remove",
            None => "broker.price-alerts",
        },
        BrokerCommand::SavingsPlans(args) => match &args.command {
            Some(BrokerSavingsPlansCommand::Add(_)) => "broker.savings-plans.add",
            Some(BrokerSavingsPlansCommand::Config(_)) => "broker.savings-plans.config",
            Some(BrokerSavingsPlansCommand::Remove(_)) => "broker.savings-plans.remove",
            None => "broker.savings-plans",
        },
        BrokerCommand::Trade(args) => broker_trade_machine_command_name(&args.command),
    }
}

fn broker_trade_machine_command_name(command: &BrokerTradeCommand) -> &'static str {
    match command {
        BrokerTradeCommand::Buy(_) => "broker.trade.buy",
        BrokerTradeCommand::Sell(_) => "broker.trade.sell",
        BrokerTradeCommand::Cancel(_) => "broker.trade.cancel",
    }
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

    use mockito::Server;
    use tempfile::tempdir;

    use super::*;
    use crate::cli::{BrokerArgs, Cli, Commands, LogoutArgs, OvernightArgs};
    use crate::config::{
        AppConfig, AuthConfig, DpopKeyBackend, EnvConfig, RuntimeAuthConfig,
        SessionBackendPreference,
    };
    use crate::machine::classify_error;
    use crate::session::{FileStore, LoginSource, Session, StorageBackend, StoredSession};

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
            trade_controls: None,
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
                mode: None,
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
    fn overnight_machine_command_name_and_json_flag_are_wired() {
        let command = Commands::Overnight(OvernightArgs {
            command: None,
            savings_account_id: Some("sav-1".to_string()),
            json: true,
        });

        assert!(command_requests_json_envelope(&command));
        assert_eq!(machine_command_name(&command), "overnight");
    }

    #[test]
    fn overnight_transactions_machine_command_name_and_json_flag_are_wired() {
        let command = Commands::Overnight(OvernightArgs {
            command: Some(OvernightCommand::Transactions(
                crate::cli::OvernightTransactionsArgs {
                    savings_account_id: Some("sav-1".to_string()),
                    page_size: 20,
                    cursor: None,
                    type_filter: Vec::new(),
                    search_term: None,
                    from_time: None,
                    to_time: None,
                    json: true,
                },
            )),
            savings_account_id: None,
            json: false,
        });

        assert!(command_requests_json_envelope(&command));
        assert_eq!(machine_command_name(&command), "overnight.transactions");
    }

    #[test]
    fn overnight_transactions_inherit_parent_selection_and_json_flags() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "overnight",
            "--savings-account-id",
            "sav-parent",
            "--json",
            "transactions",
        ]);

        let normalized = normalize_command(command).expect("normalize");
        assert!(command_requests_json_envelope(&normalized));
        assert_eq!(machine_command_name(&normalized), "overnight.transactions");
        match normalized {
            Commands::Overnight(crate::cli::OvernightArgs {
                command: Some(OvernightCommand::Transactions(args)),
                ..
            }) => {
                assert_eq!(args.savings_account_id.as_deref(), Some("sav-parent"));
                assert!(args.json);
            }
            _ => panic!("expected normalized overnight transactions command"),
        }
    }

    #[test]
    fn broker_chart_machine_command_name_and_json_flag_are_wired() {
        let command = Commands::Broker(crate::cli::BrokerArgs {
            command: BrokerCommand::Chart(crate::cli::BrokerChartArgs {
                isin: "US0378331005".to_string(),
                timeframe: crate::cli::BrokerChartTimeframe::OneMonth,
                json: true,
            }),
        });

        assert!(command_requests_json_envelope(&command));
        assert_eq!(machine_command_name(&command), "broker.chart");
    }

    #[test]
    fn broker_portfolio_groups_machine_command_name_and_json_flag_are_wired() {
        let command = Commands::Broker(crate::cli::BrokerArgs {
            command: BrokerCommand::PortfolioGroups(crate::cli::BrokerPortfolioGroupsArgs {
                command: None,
                portfolio_id: Some("portfolio-1".to_string()),
                group_id: Some("group-1".to_string()),
                json: true,
            }),
        });

        assert!(command_requests_json_envelope(&command));
        assert_eq!(machine_command_name(&command), "broker.portfolio-groups");
    }

    #[test]
    fn portfolio_groups_lifecycle_inherits_parent_flags_and_uses_leaf_command_name() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "portfolio-groups",
            "--portfolio-id",
            "p-parent",
            "--json",
            "create",
            "--name",
            "AI portfolio",
        ]);

        let normalized = normalize_command(command).expect("command should normalize");
        assert!(command_requests_json_envelope(&normalized));
        assert_eq!(
            machine_command_name(&normalized),
            "broker.portfolio-groups.create"
        );
        match normalized {
            Commands::Broker(crate::cli::BrokerArgs {
                command: BrokerCommand::PortfolioGroups(args),
            }) => match args.command {
                Some(BrokerPortfolioGroupsCommand::Create(create_args)) => {
                    assert_eq!(create_args.portfolio_id.as_deref(), Some("p-parent"));
                    assert!(create_args.json);
                }
                _ => panic!("expected create command"),
            },
            _ => panic!("expected portfolio groups command"),
        }
    }

    #[test]
    fn portfolio_groups_lifecycle_rejects_parent_group_id() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "portfolio-groups",
            "--group-id",
            "group-parent",
            "delete",
            "--group-id",
            "group-child",
        ]);

        let err = normalize_command(command).expect_err("parent group id must be rejected");

        assert_eq!(classify_error(&err).code, "broker_input_invalid");
    }

    #[test]
    fn portfolio_groups_lifecycle_inherits_parent_flags_for_every_leaf() {
        let cases: [(&[&str], &str); 5] = [
            (
                &[
                    "sc",
                    "broker",
                    "portfolio-groups",
                    "--portfolio-id",
                    "p-parent",
                    "--json",
                    "create",
                    "--name",
                    "AI portfolio",
                ],
                "broker.portfolio-groups.create",
            ),
            (
                &[
                    "sc",
                    "broker",
                    "portfolio-groups",
                    "--portfolio-id",
                    "p-parent",
                    "--json",
                    "update",
                    "--group-id",
                    "group-1",
                    "--name",
                    "Renamed",
                ],
                "broker.portfolio-groups.update",
            ),
            (
                &[
                    "sc",
                    "broker",
                    "portfolio-groups",
                    "--portfolio-id",
                    "p-parent",
                    "--json",
                    "delete",
                    "--group-id",
                    "group-1",
                ],
                "broker.portfolio-groups.delete",
            ),
            (
                &[
                    "sc",
                    "broker",
                    "portfolio-groups",
                    "--portfolio-id",
                    "p-parent",
                    "--json",
                    "assign",
                    "--group-id",
                    "group-1",
                    "--isin",
                    "US0378331005",
                ],
                "broker.portfolio-groups.assign",
            ),
            (
                &[
                    "sc",
                    "broker",
                    "portfolio-groups",
                    "--portfolio-id",
                    "p-parent",
                    "--json",
                    "unassign",
                    "--group-id",
                    "group-1",
                    "--isin",
                    "US0378331005",
                ],
                "broker.portfolio-groups.unassign",
            ),
        ];

        for (args, expected_command_name) in cases {
            let Cli { command } = Cli::parse_from(args);
            let normalized = normalize_command(command).expect("command should normalize");

            assert!(command_requests_json_envelope(&normalized));
            assert_eq!(machine_command_name(&normalized), expected_command_name);
            match normalized {
                Commands::Broker(crate::cli::BrokerArgs {
                    command: BrokerCommand::PortfolioGroups(args),
                }) => match args.command {
                    Some(BrokerPortfolioGroupsCommand::Create(args)) => {
                        assert_eq!(args.portfolio_id.as_deref(), Some("p-parent"));
                        assert!(args.json);
                    }
                    Some(BrokerPortfolioGroupsCommand::Update(args)) => {
                        assert_eq!(args.portfolio_id.as_deref(), Some("p-parent"));
                        assert!(args.json);
                    }
                    Some(BrokerPortfolioGroupsCommand::Delete(args)) => {
                        assert_eq!(args.portfolio_id.as_deref(), Some("p-parent"));
                        assert!(args.json);
                    }
                    Some(BrokerPortfolioGroupsCommand::Assign(args)) => {
                        assert_eq!(args.portfolio_id.as_deref(), Some("p-parent"));
                        assert!(args.json);
                    }
                    Some(BrokerPortfolioGroupsCommand::Unassign(args)) => {
                        assert_eq!(args.portfolio_id.as_deref(), Some("p-parent"));
                        assert!(args.json);
                    }
                    None => panic!("expected lifecycle command"),
                },
                _ => panic!("expected portfolio groups command"),
            }
        }
    }

    #[test]
    fn portfolio_groups_lifecycle_rejects_parent_group_id_for_every_leaf() {
        let cases: [&[&str]; 5] = [
            &[
                "sc",
                "broker",
                "portfolio-groups",
                "--group-id",
                "group-parent",
                "create",
                "--name",
                "AI portfolio",
            ],
            &[
                "sc",
                "broker",
                "portfolio-groups",
                "--group-id",
                "group-parent",
                "update",
                "--group-id",
                "group-child",
                "--name",
                "Renamed",
            ],
            &[
                "sc",
                "broker",
                "portfolio-groups",
                "--group-id",
                "group-parent",
                "delete",
                "--group-id",
                "group-child",
            ],
            &[
                "sc",
                "broker",
                "portfolio-groups",
                "--group-id",
                "group-parent",
                "assign",
                "--group-id",
                "group-child",
                "--isin",
                "US0378331005",
            ],
            &[
                "sc",
                "broker",
                "portfolio-groups",
                "--group-id",
                "group-parent",
                "unassign",
                "--group-id",
                "group-child",
                "--isin",
                "US0378331005",
            ],
        ];

        for args in cases {
            let Cli { command } = Cli::parse_from(args);
            let err = normalize_command(command).expect_err("parent group id must be rejected");
            assert_eq!(classify_error(&err).code, "broker_input_invalid");
        }
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

    #[test]
    fn savings_plan_config_inherits_parent_portfolio_and_json_flags() {
        let Cli { command } = Cli::parse_from([
            "sc",
            "broker",
            "savings-plans",
            "--portfolio-id",
            "p-parent",
            "--json",
            "config",
            "--isin",
            "US0378331005",
        ]);

        let normalized = normalize_command(command).expect("command should normalize");
        assert!(command_requests_json_envelope(&normalized));

        match normalized {
            Commands::Broker(BrokerArgs {
                command: BrokerCommand::SavingsPlans(args),
            }) => match args.command {
                Some(BrokerSavingsPlansCommand::Config(config_args)) => {
                    assert_eq!(config_args.portfolio_id.as_deref(), Some("p-parent"));
                    assert!(config_args.json);
                }
                _ => panic!("expected config subcommand"),
            },
            _ => panic!("expected broker savings-plans command"),
        }
    }

    #[test]
    fn capabilities_describe_savings_plan_two_phase_disclosure() {
        let capabilities = machine_capabilities(&AppConfig::default());
        let workflow = &capabilities["workflows"]["broker.savings-plans.add"];

        assert_eq!(workflow["mode"], "two_phase_confirmation");
        assert!(
            workflow["phase_1"]
                .as_str()
                .expect("phase 1 description")
                .contains("without --confirm")
        );
        assert!(
            workflow["phase_2"]
                .as_str()
                .expect("phase 2 description")
                .contains("separate interaction")
        );
        assert_eq!(
            workflow["phase_1_presentation_requirement"]["rule_id"],
            "savings_plan_ex_ante_full_disclosure_v1"
        );
        assert!(
            workflow["phase_1_presentation_requirement"]["required_leaf_paths"]
                .as_array()
                .expect("required paths")
                .iter()
                .any(|path| path == "ex_ante_costs.exitCosts.total.amount")
        );
    }
}
