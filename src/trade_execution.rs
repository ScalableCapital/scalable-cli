use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::active_session::load_active_session;
use crate::broker_shared::{checksum_for_payload, resolve_broker_ids};
use crate::config::{AppConfig, TargetEnv};
use crate::graphql::{LOCAL_READ_ONLY_ERROR_PREFIX, execute_graphql, execute_graphql_with_headers};
use crate::resolve_active_env;
use crate::session::SessionManager;
use crate::session_refresh::execute_with_refresh_retry;
use crate::trade::{
    PlaceOrderFields, SecurityTick, SingleExAnteFields, TRADE_APPROPRIATENESS_WARNING_QUERY,
    TRADE_CANCEL_ORDER_MUTATION, TRADE_PLACE_ORDER_MUTATION, TRADE_SECURITY_TICK_QUERY,
    TRADE_SINGLE_EX_ANTE_COSTS_QUERY, TRADE_TRADABILITY_QUERY, TradeComplianceDecision, TradeSide,
    TradeTradabilityGate, evaluate_trade_compliance, extract_single_trade_ex_ante_costs,
    market_buy_shares_from_amount, parse_appropriateness_warning, parse_cancel_order_result,
    parse_place_order_result, parse_security_issuer_document_links, parse_security_tick,
    parse_tradability_gate, required_non_empty, round_estimated_order_volume_for_ex_ante,
    trade_appropriateness_warning_variables, trade_cancel_order_variables,
    trade_estimated_order_price, trade_place_order_variables, trade_security_tick_variables,
    trade_side_quote_price, trade_single_ex_ante_variables, trade_tradability_variables,
};
use crate::trade_attempt::{
    load_recent_submitted_attempt, mark_failed as mark_trade_attempt_failed,
    mark_submit_in_flight as mark_trade_attempt_in_flight,
    mark_submitted as mark_trade_attempt_submitted, start_or_reuse_attempt,
};
use crate::trade_confirmation::{
    ConfirmationFields, ConfirmationPhase1Input, TradeConfirmation, load_confirmation,
    mark_confirmation_consumed, upsert_confirmation,
};
use crate::trade_controls::TradeControlsPolicy;
use crate::trade_presentation::{
    PHASE1_COMPLIANCE_RULE_ID, PHASE1_PRESENTATION_FORMAT,
    build_phase1_command_template as build_phase1_command_template_from_module,
    build_phase1_presentation as build_phase1_presentation_from_module,
    build_phase2_command_template as build_phase2_command_template_from_module,
    build_result_payload as build_result_payload_from_module,
    display_venue_label as display_venue_label_from_module,
    phase1_required_json_paths as phase1_required_json_paths_from_module,
    presentation_section_order_keys as presentation_section_order_keys_from_module,
    render_trade_buy_text as render_trade_buy_text_from_module,
    render_trade_sell_text as render_trade_sell_text_from_module,
};

const CONFIRMATION_TTL_SECONDS: i64 = 15 * 60;
const ORDER_TYPE_MARKET: &str = "market";
const ORDER_TYPE_LIMIT: &str = "limit";
const ORDER_TYPE_STOP: &str = "stop";
pub(crate) const ORDER_SIDE_BUY: &str = "buy";
pub(crate) const ORDER_SIDE_SELL: &str = "sell";
const TRADE_WARNING_LOCALE: &str = "en_DE";
const VALUE_NA: &str = "n/a";
pub(crate) const VENUE_GETTEX: &str = "GETTEX";
pub(crate) const VENUE_XETR: &str = "XETR";
pub(crate) const VENUE_SEIX: &str = "SEIX";
pub(crate) const VENUE_LABEL_GETTEX: &str = "Börse München (gettex)";
pub(crate) const VENUE_LABEL_XETRA: &str = "Xetra";
pub(crate) const VENUE_LABEL_SEIX: &str = "European Investor Exchange (EIX)";

#[derive(Debug, Clone)]
pub(crate) struct TradeIntent {
    pub(crate) side: TradeSide,
    pub(crate) isin: String,
    pub(crate) amount: Option<f64>,
    pub(crate) amount_str: Option<String>,
    pub(crate) shares: Option<f64>,
    pub(crate) shares_str: Option<String>,
    pub(crate) order_type: String,
    pub(crate) limit_price: Option<f64>,
    pub(crate) stop_price: Option<f64>,
    pub(crate) limit_price_str: Option<String>,
    pub(crate) stop_price_str: Option<String>,
    pub(crate) venue_override: Option<String>,
    pub(crate) locale: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedTrade {
    pub(crate) env: TargetEnv,
    pub(crate) account_id: String,
    pub(crate) portfolio_id: String,
    pub(crate) account_source: &'static str,
    pub(crate) portfolio_source: &'static str,
    pub(crate) intent: TradeIntent,
    pub(crate) tradability_gate: TradeTradabilityGate,
    pub(crate) compliance_decision: TradeComplianceDecision,
    pub(crate) warning_json: Value,
    pub(crate) warning_version_for_order: Option<String>,
    pub(crate) appropriateness_id_for_order: Option<String>,
    pub(crate) quote_mid_price_value: f64,
    pub(crate) quote_ask_price_value: Option<f64>,
    pub(crate) quote_bid_price_value: Option<f64>,
    pub(crate) quote_mid_price: String,
    pub(crate) quote_ask_price: Option<String>,
    pub(crate) quote_bid_price: Option<String>,
    pub(crate) quote_currency: String,
    pub(crate) quote_timestamp_utc: Option<String>,
    pub(crate) selected_venue_label: String,
    pub(crate) primary_kid_url: Option<String>,
    pub(crate) secondary_kid_url: Option<String>,
    pub(crate) sizing_price_basis: &'static str,
    pub(crate) sizing_price: String,
    pub(crate) estimate_price_basis: &'static str,
    pub(crate) estimate_price: String,
    pub(crate) number_of_shares: f64,
    pub(crate) number_of_shares_str: String,
    pub(crate) is_whole_position_sold: bool,
    pub(crate) estimated_order_volume_raw: f64,
    pub(crate) estimated_order_volume: f64,
    pub(crate) ex_ante_costs: Value,
    pub(crate) confirmation_fields: ConfirmationFields,
    pub(crate) snapshot_payload: Value,
    pub(crate) intent_checksum: String,
}

impl TradeIntent {
    pub(crate) fn side_label(&self) -> &'static str {
        match self.side {
            TradeSide::Buy => ORDER_SIDE_BUY,
            TradeSide::Sell => ORDER_SIDE_SELL,
        }
    }
}

#[derive(Debug, Clone)]
struct Phase2Input {
    confirmation_id: String,
    accept_unsuitable: bool,
    isin: String,
    amount: Option<String>,
    amount_value: Option<f64>,
    shares: Option<String>,
    shares_value: Option<f64>,
    venue: Option<String>,
    order_type: String,
    limit_price: Option<String>,
    stop_price: Option<String>,
    limit_price_value: Option<f64>,
    stop_price_value: Option<f64>,
}

#[derive(Debug, Clone)]
struct ValidatedOrderPrices {
    limit_price: Option<String>,
    stop_price: Option<String>,
    limit_price_value: Option<f64>,
    stop_price_value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
struct TradeCalculation {
    sizing_price_basis: &'static str,
    sizing_price_value: f64,
    estimate_price_basis: &'static str,
    estimate_price_value: f64,
    number_of_shares: f64,
    number_of_shares_str: String,
    is_whole_position_sold: bool,
    estimated_order_volume_raw: f64,
    estimated_order_volume: f64,
}

pub(crate) fn execute_broker_trade_buy(
    args: crate::cli::BrokerTradeBuyArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    if args.confirm.is_some() {
        execute_trade_buy_phase2(args, config, session_manager)
    } else {
        execute_trade_buy_phase1(args, config, session_manager)
    }
}

pub(crate) fn execute_broker_trade_sell(
    args: crate::cli::BrokerTradeSellArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    if args.confirm.is_some() {
        execute_trade_sell_phase2(args, config, session_manager)
    } else {
        execute_trade_sell_phase1(args, config, session_manager)
    }
}

pub(crate) fn execute_broker_trade_cancel(
    args: crate::cli::BrokerTradeCancelArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let order_id = required_non_empty(&args.order_id, "order_id")?;
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
    let variables = trade_cancel_order_variables(&ids.portfolio_id, &order_id)?;
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
                TRADE_CANCEL_ORDER_MUTATION,
                &variables,
                Some("BrokerCancelOrder"),
                access_context,
                dpop_options,
            )
        },
    )?;
    parse_cancel_order_result(&response)?;

    Ok(json!({
        "account_id": ids.account_id,
        "portfolio_id": ids.portfolio_id,
        "resolution": {
            "account": ids.account_source,
            "portfolio": ids.portfolio_source,
        },
        "result": {
            "order_id": order_id,
            "accepted": true,
        },
    }))
}

fn execute_trade_buy_phase1(
    args: crate::cli::BrokerTradeBuyArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let phase1_input = parse_phase1_input_for_confirmation_buy(&args)?;
    let intent = parse_phase1_intent_buy(&args)?;
    execute_trade_phase1(
        TradeSide::Buy,
        phase1_input,
        intent,
        args.json,
        config,
        session_manager,
    )
}

fn execute_trade_buy_phase2(
    args: crate::cli::BrokerTradeBuyArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let phase2 = parse_phase2_input_buy(&args)?;
    execute_trade_phase2(TradeSide::Buy, phase2, config, session_manager)
}

fn execute_trade_sell_phase1(
    args: crate::cli::BrokerTradeSellArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let phase1_input = parse_phase1_input_for_confirmation_sell(&args)?;
    let intent = parse_phase1_intent_sell(&args)?;
    execute_trade_phase1(
        TradeSide::Sell,
        phase1_input,
        intent,
        args.json,
        config,
        session_manager,
    )
}

fn execute_trade_sell_phase2(
    args: crate::cli::BrokerTradeSellArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let phase2 = parse_phase2_input_sell(&args)?;
    execute_trade_phase2(TradeSide::Sell, phase2, config, session_manager)
}

fn execute_trade_phase1(
    side: TradeSide,
    phase1_input: ConfirmationPhase1Input,
    intent: TradeIntent,
    json_mode: bool,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let trade_controls = TradeControlsPolicy::from_app_config(config);
    trade_controls.check_isin(&intent.isin)?;
    let prepared = prepare_trade(&intent, config, session_manager)?;
    trade_controls.check_estimated_order_volume(prepared.estimated_order_volume)?;
    ensure_price_warning_quote_legs(
        &prepared.intent,
        &SecurityTick {
            ask_price: prepared.quote_ask_price_value,
            bid_price: prepared.quote_bid_price_value,
            mid_price: prepared.quote_mid_price_value,
            currency: prepared.quote_currency.clone(),
            is_outdated: false,
            timestamp_utc: prepared.quote_timestamp_utc.clone(),
        },
    )?;

    let now = current_epoch_seconds();
    let nonce = format!("{:032x}", rand::random::<u128>());
    let confirmation_id =
        confirmation_id_from_nonce_and_checksum(&nonce, &prepared.intent_checksum);
    let expires_at_epoch = now + CONFIRMATION_TTL_SECONDS;

    let confirmation = TradeConfirmation {
        confirmation_id: confirmation_id.clone(),
        intent_checksum: prepared.intent_checksum.clone(),
        nonce,
        created_at_epoch: now,
        expires_at_epoch,
        consumed_at_epoch: None,
        env: prepared.env.as_str().to_string(),
        account_id: prepared.account_id.clone(),
        portfolio_id: prepared.portfolio_id.clone(),
        side: side_label(side).to_string(),
        order_type: prepared.intent.order_type.clone(),
        locale: prepared.intent.locale.clone(),
        venue_override: prepared.intent.venue_override.clone(),
        warning_version: prepared.warning_version_for_order.clone(),
        requires_accept_unsuitable: phase1_requires_accept_unsuitable(&prepared),
        phase1_input: phase1_input.clone(),
        fields: prepared.confirmation_fields.clone(),
        snapshot_payload: prepared.snapshot_payload.clone(),
        ex_ante_costs: prepared.ex_ante_costs.clone(),
    };
    upsert_confirmation(confirmation, now).context("Failed to persist phase 1 confirmation")?;

    let phase1_command_template_json = build_phase1_command_template(&phase1_input, true);
    let requires_accept_unsuitable = phase1_requires_accept_unsuitable(&prepared);
    let phase2_command_template_json = build_phase2_command_template(
        &confirmation_id,
        &phase1_input,
        requires_accept_unsuitable,
        true,
    );
    let command_template = build_phase2_command_template(
        &confirmation_id,
        &phase1_input,
        requires_accept_unsuitable,
        json_mode,
    );

    let result_payload = build_result_payload(&prepared, None);
    let confirmation_payload = json!({
        "id": confirmation_id,
        "intent_checksum": prepared.intent_checksum,
        "expires_at_epoch": expires_at_epoch,
        "required_fields": prepared.confirmation_fields,
        "warning_version": prepared.warning_version_for_order,
        "requires_accept_unsuitable": requires_accept_unsuitable,
        "accept_unsuitable_flag": requires_accept_unsuitable.then_some("--accept-unsuitable"),
        "command_template": command_template,
        "phase_1_command_template_json": phase1_command_template_json,
        "phase_2_command_template_json": phase2_command_template_json,
    });
    let presentation = build_phase1_presentation(&result_payload, &confirmation_payload)?;
    let required_json_paths = phase1_required_json_paths(side);
    let presentation_section_order = presentation_section_order_keys(side);
    let presentation_required_paths = presentation_required_paths_from_presentation(&presentation)?;

    Ok(json!({
        "account_id": prepared.account_id,
        "portfolio_id": prepared.portfolio_id,
        "resolution": {
            "account": prepared.account_source,
            "portfolio": prepared.portfolio_source,
        },
        "result": result_payload,
        "confirmation": confirmation_payload,
        "compliance": {
            "rule_id": PHASE1_COMPLIANCE_RULE_ID,
            "must_present_all_information": true,
            "instruction": "Before running phase 2, you MUST present all pre-trade information from phase 1 in a human-readable summary (not raw JSON), without omitting or changing any values. After presenting it, you MUST explicitly ask the user whether to proceed and receive an explicit affirmative confirmation from the user in a separate interaction step. You MUST NOT execute phase 2 automatically, implicitly, or in the same step as phase 1 output.",
            "requires_explicit_user_confirmation_between_phases": true,
            "forbid_automatic_phase_2_execution": true,
        "confirmation_must_be_separate_step": true,
        "required_json_paths": required_json_paths,
            "presentation": {
                "format": PHASE1_PRESENTATION_FORMAT,
                "section_order": presentation_section_order,
                "required_leaf_paths": presentation_required_paths,
                "preserve_exact_values": true,
                "display_null_as_literal": true,
                "raw_json_only_on_user_request": true,
            },
            "phase_2_requirement": if requires_accept_unsuitable {
                "Repeat the phase 1 command with --confirm <id> and --accept-unsuitable only after explicit affirmative user confirmation in a separate interaction step."
            } else {
                "Repeat the phase 1 command with --confirm <id> only after explicit affirmative user confirmation in a separate interaction step."
            }
        },
        "presentation": presentation,
        "next_step": "confirm_with_id"
    }))
}

fn execute_trade_phase2(
    side: TradeSide,
    phase2: Phase2Input,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let now = current_epoch_seconds();

    let stored = load_confirmation(&phase2.confirmation_id)?.ok_or_else(|| {
        anyhow!(
            "CONFIRMATION_NOT_FOUND: no phase 1 snapshot found for id '{}'",
            phase2.confirmation_id
        )
    })?;

    if stored.consumed_at_epoch.is_some() {
        bail!(
            "CONFIRMATION_ALREADY_USED: confirmation id '{}' was already consumed",
            phase2.confirmation_id
        );
    }
    if now > stored.expires_at_epoch {
        bail!(
            "CONFIRMATION_EXPIRED: confirmation id '{}' expired at epoch {}",
            phase2.confirmation_id,
            stored.expires_at_epoch
        );
    }

    let active_env = resolve_active_env(session_manager)?;
    if stored.env != active_env.as_str() {
        bail!(
            "CONFIRMATION_ENV_MISMATCH: confirmation env '{}' does not match active env '{}'",
            stored.env,
            active_env.as_str()
        );
    }

    assert_phase2_matches_phase1_input(side, &phase2, &stored.phase1_input)?;

    let intent = build_phase2_intent(side, &phase2, &stored.phase1_input)?;
    let trade_controls = TradeControlsPolicy::from_app_config(config);
    trade_controls.check_isin(&intent.isin)?;
    let prepared = prepare_trade(&intent, config, session_manager)?;

    ensure_phase2_submission_requirements(&prepared, &phase2, &stored)?;
    trade_controls.check_estimated_order_volume(prepared.estimated_order_volume)?;

    let order_submission = submit_order(
        &prepared,
        &stored.confirmation_id,
        stored.expires_at_epoch,
        config,
        session_manager,
    )?;
    mark_confirmation_consumed(&phase2.confirmation_id, current_epoch_seconds())
        .context("Failed to mark confirmation token as consumed")?;

    Ok(json!({
        "account_id": prepared.account_id,
        "portfolio_id": prepared.portfolio_id,
        "resolution": {
            "account": prepared.account_source,
            "portfolio": prepared.portfolio_source,
        },
        "result": build_result_payload(&prepared, Some(order_submission)),
        "confirmation": {
            "id": phase2.confirmation_id,
            "intent_checksum": prepared.intent_checksum,
            "consumed": true,
        },
        "next_step": "completed"
    }))
}

fn ensure_price_warning_quote_legs(intent: &TradeIntent, quote: &SecurityTick) -> Result<()> {
    match intent.side {
        TradeSide::Buy
            if intent.limit_price.is_some()
                && intent.stop_price.is_none()
                && quote.ask_price.is_none() =>
        {
            bail!(
                "PRICE_WARNING_UNAVAILABLE: missing askPrice required for buy limit warning evaluation"
            );
        }
        TradeSide::Sell
            if intent.limit_price.is_some()
                && intent.stop_price.is_none()
                && quote.bid_price.is_none() =>
        {
            bail!(
                "PRICE_WARNING_UNAVAILABLE: missing bidPrice required for sell limit warning evaluation"
            );
        }
        _ => {}
    }

    Ok(())
}

fn ensure_phase2_snapshot_matches(
    prepared: &PreparedTrade,
    stored: &TradeConfirmation,
) -> Result<()> {
    if prepared.confirmation_fields != stored.fields {
        bail!(
            "CONFIRMATION_FIELDS_MISMATCH: fresh pre-trade values differ from phase 1 snapshot; rerun phase 1"
        );
    }
    if canonical_ex_ante_amounts(&prepared.ex_ante_costs)
        != canonical_ex_ante_amounts(&stored.ex_ante_costs)
    {
        bail!(
            "CONFIRMATION_FIELDS_MISMATCH: fresh ex-ante costs differ from phase 1 snapshot; rerun phase 1"
        );
    }
    if prepared.intent_checksum != stored.intent_checksum {
        bail!(
            "CONFIRMATION_FIELDS_MISMATCH: fresh intent checksum differs from phase 1 snapshot; rerun phase 1"
        );
    }
    if prepared.warning_version_for_order != stored.warning_version {
        bail!(
            "CONFIRMATION_FIELDS_MISMATCH: fresh warning version differs from phase 1 snapshot; rerun phase 1"
        );
    }
    if phase1_requires_accept_unsuitable(prepared) != stored.requires_accept_unsuitable {
        bail!(
            "CONFIRMATION_FIELDS_MISMATCH: fresh suitability acknowledgement requirement differs from phase 1 snapshot; rerun phase 1"
        );
    }

    Ok(())
}

fn ensure_phase2_submission_requirements(
    prepared: &PreparedTrade,
    phase2: &Phase2Input,
    stored: &TradeConfirmation,
) -> Result<()> {
    ensure_phase2_snapshot_matches(prepared, stored)?;
    ensure_unsuitable_acknowledgement(phase2, stored)?;
    Ok(())
}

fn canonical_ex_ante_amounts(ex_ante_costs: &Value) -> BTreeMap<String, String> {
    let mut amounts = BTreeMap::new();
    collect_canonical_ex_ante_amounts(ex_ante_costs, "", &mut amounts);
    amounts
}

fn collect_canonical_ex_ante_amounts(
    value: &Value,
    path: &str,
    amounts: &mut BTreeMap<String, String>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    format!("/{key}")
                } else {
                    format!("{path}/{key}")
                };

                if key == "amount" {
                    amounts.insert(child_path, canonical_cost_from_json(Some(child)));
                } else {
                    collect_canonical_ex_ante_amounts(child, &child_path, amounts);
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let child_path = if path.is_empty() {
                    format!("/{index}")
                } else {
                    format!("{path}/{index}")
                };
                collect_canonical_ex_ante_amounts(child, &child_path, amounts);
            }
        }
        _ => {}
    }
}

fn build_phase2_intent(
    side: TradeSide,
    phase2: &Phase2Input,
    _phase1: &ConfirmationPhase1Input,
) -> Result<TradeIntent> {
    Ok(TradeIntent {
        side,
        isin: canonical_isin(&phase2.isin, "isin")?,
        amount: phase2.amount_value,
        amount_str: phase2.amount.clone(),
        shares: phase2.shares_value,
        shares_str: phase2.shares.clone(),
        order_type: phase2.order_type.clone(),
        limit_price: phase2.limit_price_value,
        stop_price: phase2.stop_price_value,
        limit_price_str: phase2.limit_price.clone(),
        stop_price_str: phase2.stop_price.clone(),
        venue_override: phase2.venue.as_deref().map(str::to_uppercase),
        locale: TRADE_WARNING_LOCALE.to_string(),
    })
}

fn parse_phase1_intent_buy(args: &crate::cli::BrokerTradeBuyArgs) -> Result<TradeIntent> {
    let isin = canonical_isin(required_input_value(args.isin.as_deref(), "isin")?, "isin")?;
    let amount_raw = required_input_value(args.amount.as_deref(), "amount")?;
    let (amount, amount_str) = parse_positive_decimal_from_str(amount_raw, "amount")?;
    let prices = validate_order_price_flags(
        args.order_type,
        args.limit_price.as_deref(),
        args.stop_price.as_deref(),
    )?;

    let venue_override = args
        .venue
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_uppercase());
    Ok(TradeIntent {
        side: TradeSide::Buy,
        isin,
        amount: Some(amount),
        amount_str: Some(amount_str),
        shares: None,
        shares_str: None,
        order_type: order_type_label(args.order_type).to_string(),
        limit_price: prices.limit_price_value,
        stop_price: prices.stop_price_value,
        limit_price_str: prices.limit_price,
        stop_price_str: prices.stop_price,
        venue_override,
        locale: TRADE_WARNING_LOCALE.to_string(),
    })
}

fn parse_phase1_intent_sell(args: &crate::cli::BrokerTradeSellArgs) -> Result<TradeIntent> {
    let isin = canonical_isin(required_input_value(args.isin.as_deref(), "isin")?, "isin")?;
    let shares_raw = required_input_value(args.shares.as_deref(), "shares")?;
    let (shares, shares_str) = parse_positive_decimal_from_str(shares_raw, "shares")?;
    let prices = validate_order_price_flags(
        args.order_type,
        args.limit_price.as_deref(),
        args.stop_price.as_deref(),
    )?;

    let venue_override = args
        .venue
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_uppercase());
    Ok(TradeIntent {
        side: TradeSide::Sell,
        isin,
        amount: None,
        amount_str: None,
        shares: Some(shares),
        shares_str: Some(shares_str),
        order_type: order_type_label(args.order_type).to_string(),
        limit_price: prices.limit_price_value,
        stop_price: prices.stop_price_value,
        limit_price_str: prices.limit_price,
        stop_price_str: prices.stop_price,
        venue_override,
        locale: TRADE_WARNING_LOCALE.to_string(),
    })
}

fn parse_phase1_input_for_confirmation_buy(
    args: &crate::cli::BrokerTradeBuyArgs,
) -> Result<ConfirmationPhase1Input> {
    let isin = required_input_value(args.isin.as_deref(), "isin")?.to_string();
    let amount = required_input_value(args.amount.as_deref(), "amount")?.to_string();
    let _ = parse_positive_decimal_from_str(&amount, "amount")?;
    let prices = validate_order_price_flags(
        args.order_type,
        args.limit_price.as_deref(),
        args.stop_price.as_deref(),
    )?;

    Ok(ConfirmationPhase1Input {
        side: ORDER_SIDE_BUY.to_string(),
        isin,
        amount: Some(amount),
        shares: None,
        venue: optional_trimmed_string(args.venue.as_deref()),
        order_type: order_type_label(args.order_type).to_string(),
        limit_price: prices.limit_price,
        stop_price: prices.stop_price,
    })
}

fn parse_phase1_input_for_confirmation_sell(
    args: &crate::cli::BrokerTradeSellArgs,
) -> Result<ConfirmationPhase1Input> {
    let isin = required_input_value(args.isin.as_deref(), "isin")?.to_string();
    let shares = required_input_value(args.shares.as_deref(), "shares")?.to_string();
    let _ = parse_positive_decimal_from_str(&shares, "shares")?;
    let prices = validate_order_price_flags(
        args.order_type,
        args.limit_price.as_deref(),
        args.stop_price.as_deref(),
    )?;

    Ok(ConfirmationPhase1Input {
        side: ORDER_SIDE_SELL.to_string(),
        isin,
        amount: None,
        shares: Some(shares),
        venue: optional_trimmed_string(args.venue.as_deref()),
        order_type: order_type_label(args.order_type).to_string(),
        limit_price: prices.limit_price,
        stop_price: prices.stop_price,
    })
}

fn parse_phase2_input_buy(args: &crate::cli::BrokerTradeBuyArgs) -> Result<Phase2Input> {
    let confirmation_id = required_flag_value(args.confirm.as_deref(), "--confirm")?.to_string();
    let isin = required_input_value(args.isin.as_deref(), "isin")?.to_string();
    let amount = required_input_value(args.amount.as_deref(), "amount")?.to_string();
    let amount_value = parse_positive_decimal_from_str(&amount, "amount")?.0;
    let prices = validate_order_price_flags(
        args.order_type,
        args.limit_price.as_deref(),
        args.stop_price.as_deref(),
    )?;
    let venue = optional_trimmed_string(args.venue.as_deref());
    let order_type = order_type_label(args.order_type).to_string();

    Ok(Phase2Input {
        confirmation_id,
        accept_unsuitable: args.accept_unsuitable,
        isin,
        amount: Some(amount),
        amount_value: Some(amount_value),
        shares: None,
        shares_value: None,
        venue,
        order_type,
        limit_price: prices.limit_price,
        stop_price: prices.stop_price,
        limit_price_value: prices.limit_price_value,
        stop_price_value: prices.stop_price_value,
    })
}

fn parse_phase2_input_sell(args: &crate::cli::BrokerTradeSellArgs) -> Result<Phase2Input> {
    let confirmation_id = required_flag_value(args.confirm.as_deref(), "--confirm")?.to_string();
    let isin = required_input_value(args.isin.as_deref(), "isin")?.to_string();
    let shares = required_input_value(args.shares.as_deref(), "shares")?.to_string();
    let shares_value = parse_positive_decimal_from_str(&shares, "shares")?.0;
    let prices = validate_order_price_flags(
        args.order_type,
        args.limit_price.as_deref(),
        args.stop_price.as_deref(),
    )?;
    let venue = optional_trimmed_string(args.venue.as_deref());
    let order_type = order_type_label(args.order_type).to_string();

    Ok(Phase2Input {
        confirmation_id,
        accept_unsuitable: false,
        isin,
        amount: None,
        amount_value: None,
        shares: Some(shares),
        shares_value: Some(shares_value),
        venue,
        order_type,
        limit_price: prices.limit_price,
        stop_price: prices.stop_price,
        limit_price_value: prices.limit_price_value,
        stop_price_value: prices.stop_price_value,
    })
}

fn ensure_unsuitable_acknowledgement(
    phase2: &Phase2Input,
    stored: &TradeConfirmation,
) -> Result<()> {
    if stored.requires_accept_unsuitable && !phase2.accept_unsuitable {
        bail!(
            "CONFIRMATION_UNSUITABLE_ACK_REQUIRED: phase 1 marked this instrument as not suitable; rerun phase 2 with --accept-unsuitable"
        );
    }
    Ok(())
}

fn assert_phase2_matches_phase1_input(
    side: TradeSide,
    phase2: &Phase2Input,
    phase1: &ConfirmationPhase1Input,
) -> Result<()> {
    if phase1.side != side_label(side) {
        bail!("CONFIRMATION_FIELDS_MISMATCH: --side does not match phase 1 input");
    }
    if phase2.isin != phase1.isin {
        bail!("CONFIRMATION_FIELDS_MISMATCH: --isin does not match phase 1 input");
    }
    if phase2.amount != phase1.amount {
        bail!("CONFIRMATION_FIELDS_MISMATCH: --amount does not match phase 1 input");
    }
    if phase2.shares != phase1.shares {
        bail!("CONFIRMATION_FIELDS_MISMATCH: --shares does not match phase 1 input");
    }
    if phase2.venue != phase1.venue {
        bail!("CONFIRMATION_FIELDS_MISMATCH: --venue does not match phase 1 input");
    }
    if phase2.order_type != phase1.order_type {
        bail!("CONFIRMATION_FIELDS_MISMATCH: --order-type does not match phase 1 input");
    }
    if phase2.limit_price != phase1.limit_price {
        bail!("CONFIRMATION_FIELDS_MISMATCH: --limit-price does not match phase 1 input");
    }
    if phase2.stop_price != phase1.stop_price {
        bail!("CONFIRMATION_FIELDS_MISMATCH: --stop-price does not match phase 1 input");
    }
    Ok(())
}

fn prepare_trade(
    intent: &TradeIntent,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<PreparedTrade> {
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
        None,
    )?;

    let tradability_variables =
        trade_tradability_variables(&ids.account_id, &ids.portfolio_id, &intent.isin)?;
    let tradability_response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                TRADE_TRADABILITY_QUERY,
                &tradability_variables,
                Some("getTradingTradability"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let tradability_gate = parse_tradability_gate(
        &tradability_response,
        intent.side,
        intent.venue_override.as_deref(),
    )?;
    if !tradability_gate.tradable {
        let reason = tradability_gate
            .selected_venue_unavailability_reason
            .as_deref()
            .unwrap_or("No reason provided by backend");
        bail!(
            "TRADE_NOT_TRADABLE: {} trading is not available on venue '{}' (status={}): {}",
            side_label(intent.side),
            tradability_gate.selected_venue,
            tradability_gate.selected_venue_status,
            reason
        );
    }

    let mut compliance_decision = TradeComplianceDecision::not_required();
    let mut warning_json = Value::Null;
    let mut warning_version_for_order = None::<String>;
    let mut appropriateness_id_for_order = None::<String>;

    if intent.side.is_buy() {
        compliance_decision = evaluate_trade_compliance(
            &tradability_response,
            intent.side,
            tradability_gate.requires_appropriateness,
        )?;
        appropriateness_id_for_order = compliance_decision.submission_appropriateness_id.clone();

        if compliance_decision.questionnaire_required {
            let suitability_type = questionnaire_name_for_error(&compliance_decision);
            let status = compliance_decision.status.as_str();
            let reason = compliance_decision
                .questionnaire_reason
                .unwrap_or("unknown");
            bail!(
                "SUITABILITY_QUESTIONNAIRE_REQUIRED: complete the required {} questionnaire outside the CLI before trading (status={}, reason={})",
                suitability_type,
                status,
                reason,
            );
        }

        if matches!(
            compliance_decision.warning.kind,
            crate::trade::TradeWarningKind::LegacyAppropriatenessWarning
        ) {
            let warning_variables = trade_appropriateness_warning_variables(&intent.locale)?;
            let warning_response = execute_with_refresh_retry(
                session_manager,
                env,
                &env_cfg,
                &mut session,
                dpop_options,
                |token| {
                    execute_graphql(
                        &env_cfg.graphql_url,
                        token,
                        TRADE_APPROPRIATENESS_WARNING_QUERY,
                        &warning_variables,
                        Some("getBrokerAppropriatenessWarning"),
                        access_context,
                        dpop_options,
                    )
                },
            )?;
            let warning = parse_appropriateness_warning(&warning_response)?;
            warning_version_for_order = Some(warning.version.clone());
            warning_json = json!({
                "kind": compliance_decision.warning.kind.as_str(),
                "title": Value::Null,
                "body": warning.prompt_text,
                "locale": warning.locale,
                "version": warning.version,
                "acknowledgement_text": warning.acknowledgement_text,
            });
        } else if !matches!(
            compliance_decision.warning.kind,
            crate::trade::TradeWarningKind::None
        ) {
            warning_json = build_warning_payload(&compliance_decision);
        }
    }

    let quote_variables =
        trade_security_tick_variables(&ids.account_id, &ids.portfolio_id, &intent.isin)?;
    let quote_response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                TRADE_SECURITY_TICK_QUERY,
                &quote_variables,
                Some("getSecurityTick"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let quote = parse_security_tick(&quote_response)?;
    if quote.is_outdated {
        bail!(
            "EX_ANTE_COST_UNAVAILABLE: security quote is outdated, cannot continue with ex-ante disclosure"
        );
    }
    let issuer_document_links =
        parse_security_issuer_document_links(&quote_response, &intent.locale)?;
    let calculation = calculate_trade_quantities(intent, &quote, &tradability_gate)?;

    let ex_ante_variables = trade_single_ex_ante_variables(SingleExAnteFields {
        person_id: &ids.account_id,
        portfolio_id: &ids.portfolio_id,
        isin: &intent.isin,
        side: intent.side,
        estimated_order_volume: calculation.estimated_order_volume,
        number_of_shares: calculation.number_of_shares,
        venue: &tradability_gate.selected_venue,
        is_whole_position_sold: calculation.is_whole_position_sold,
    })?;
    let ex_ante_response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                TRADE_SINGLE_EX_ANTE_COSTS_QUERY,
                &ex_ante_variables,
                Some("getSingleTradeExAnteCost"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let ex_ante_costs = extract_single_trade_ex_ante_costs(&ex_ante_response)?;
    let quote_mid_price = canonical_decimal_from_f64(quote.mid_price);
    let quote_ask_price = quote.ask_price.map(canonical_decimal_from_f64);
    let quote_bid_price = quote.bid_price.map(canonical_decimal_from_f64);
    let sizing_price_formatted = canonical_decimal_from_f64(calculation.sizing_price_value);
    let estimate_price_formatted = canonical_decimal_from_f64(calculation.estimate_price_value);
    let selected_venue_label = display_venue_label(&tradability_gate.selected_venue);

    let confirmation_fields = ConfirmationFields {
        isin: intent.isin.clone(),
        amount: intent.amount_str.clone(),
        currency: canonical_required_upper(&quote.currency, "currency")?,
        venue: canonical_required_upper(&tradability_gate.selected_venue, "venue")?,
        shares: calculation.number_of_shares_str.clone(),
        entry_total: canonical_cost_from_json(ex_ante_costs.pointer("/entryCosts/total/amount")),
        ongoing_total: canonical_cost_from_json(
            ex_ante_costs.pointer("/ongoingCosts/total/amount"),
        ),
        exit_total: canonical_cost_from_json(ex_ante_costs.pointer("/exitCosts/total/amount")),
        five_years_total: canonical_cost_from_json(ex_ante_costs.pointer("/fiveYearsCosts/amount")),
    };

    let snapshot_payload = json!({
        "env": env.as_str(),
        "account_id": &ids.account_id,
        "portfolio_id": &ids.portfolio_id,
        "intent": {
            "side": side_label(intent.side),
            "isin": &intent.isin,
            "amount": &intent.amount_str,
            "shares": &intent.shares_str,
            "order_type": &intent.order_type,
            "limit_price": &intent.limit_price_str,
            "stop_price": &intent.stop_price_str,
            "venue_override": &intent.venue_override,
            "locale": &intent.locale,
        },
        "market_quote": {
            "mid_price": &quote_mid_price,
            "ask_price": &quote_ask_price,
            "bid_price": &quote_bid_price,
            "currency": &confirmation_fields.currency,
            "is_outdated": quote.is_outdated,
            "timestamp_utc": &quote.timestamp_utc,
        },
        "calculation": {
            "sizing_price_basis": calculation.sizing_price_basis,
            "sizing_price": &sizing_price_formatted,
            "estimate_price_basis": calculation.estimate_price_basis,
            "estimate_price": &estimate_price_formatted,
            "shares": &calculation.number_of_shares_str,
            "estimated_order_volume_raw": canonical_decimal_from_f64(
                calculation.estimated_order_volume_raw,
            ),
            "estimated_order_volume": canonical_decimal_from_f64(
                calculation.estimated_order_volume,
            ),
            "is_whole_position_sold": calculation.is_whole_position_sold,
        },
        "tradability": {
            "status": &tradability_gate.status,
            "selected_venue": &tradability_gate.selected_venue,
            "selected_venue_status": &tradability_gate.selected_venue_status,
            "selected_venue_unavailability_reason": &tradability_gate.selected_venue_unavailability_reason,
            "requires_appropriateness": tradability_gate.requires_appropriateness,
            "tradable": tradability_gate.tradable,
        },
        "suitability": build_suitability_payload(&compliance_decision),
        "warning": &warning_json,
        "warning_version": &warning_version_for_order,
        "ex_ante_costs": &ex_ante_costs,
        "confirmation_fields": &confirmation_fields,
    });
    let intent_checksum = checksum_for_payload(&intent_checksum_payload(IntentChecksumInput {
        env,
        account_id: &ids.account_id,
        portfolio_id: &ids.portfolio_id,
        side: side_label(intent.side),
        order_type: &intent.order_type,
        isin: &intent.isin,
        amount: intent.amount_str.as_deref(),
        shares: intent.shares_str.as_deref(),
        limit_price: intent.limit_price_str.as_deref(),
        stop_price: intent.stop_price_str.as_deref(),
        venue_override: intent.venue_override.as_deref(),
    }));

    Ok(PreparedTrade {
        env,
        account_id: ids.account_id,
        portfolio_id: ids.portfolio_id,
        account_source: ids.account_source,
        portfolio_source: ids.portfolio_source,
        intent: intent.clone(),
        tradability_gate,
        compliance_decision,
        warning_json,
        warning_version_for_order,
        appropriateness_id_for_order,
        quote_mid_price_value: quote.mid_price,
        quote_ask_price_value: quote.ask_price,
        quote_bid_price_value: quote.bid_price,
        quote_mid_price,
        quote_ask_price,
        quote_bid_price,
        quote_currency: quote.currency,
        quote_timestamp_utc: quote.timestamp_utc,
        selected_venue_label,
        primary_kid_url: issuer_document_links.primary_kid_url,
        secondary_kid_url: issuer_document_links.secondary_kid_url,
        sizing_price_basis: calculation.sizing_price_basis,
        sizing_price: sizing_price_formatted,
        estimate_price_basis: calculation.estimate_price_basis,
        estimate_price: estimate_price_formatted,
        number_of_shares: calculation.number_of_shares,
        number_of_shares_str: calculation.number_of_shares_str,
        is_whole_position_sold: calculation.is_whole_position_sold,
        estimated_order_volume_raw: calculation.estimated_order_volume_raw,
        estimated_order_volume: calculation.estimated_order_volume,
        ex_ante_costs,
        confirmation_fields,
        snapshot_payload,
        intent_checksum,
    })
}

fn calculate_trade_quantities(
    intent: &TradeIntent,
    quote: &SecurityTick,
    tradability_gate: &TradeTradabilityGate,
) -> Result<TradeCalculation> {
    let sizing_price = trade_side_quote_price(intent.side, quote)?;
    let estimate_price = trade_estimated_order_price(
        intent.side,
        &intent.order_type,
        intent.limit_price,
        intent.stop_price,
        quote,
    )?;

    let (number_of_shares, number_of_shares_str, is_whole_position_sold) = match intent.side {
        TradeSide::Buy => {
            let amount = intent.amount.ok_or_else(|| {
                anyhow!("Trade input invalid: field 'amount' must be present for buy flow")
            })?;
            let amount_str = intent.amount_str.as_deref().unwrap_or(VALUE_NA);
            let shares_u64 = market_buy_shares_from_amount(amount, sizing_price.price, false);
            if shares_u64 == 0 {
                bail!(
                    "EX_ANTE_COST_UNAVAILABLE: amount {} {} is below one share at current {} {}",
                    amount_str,
                    quote.currency,
                    pricing_basis_display_name(sizing_price.basis),
                    canonical_decimal_from_f64(sizing_price.price)
                );
            }
            (shares_u64 as f64, shares_u64.to_string(), false)
        }
        TradeSide::Sell => {
            let shares = intent.shares.ok_or_else(|| {
                anyhow!("Trade input invalid: field 'shares' must be present for sell flow")
            })?;
            resolve_sell_trade_shares(
                shares,
                intent.shares_str.as_deref(),
                &tradability_gate.selected_venue,
                tradability_gate.selected_venue_sellable,
            )?
        }
    };

    let estimated_order_volume_raw = number_of_shares * estimate_price.price;
    let estimated_order_volume =
        round_estimated_order_volume_for_ex_ante(estimated_order_volume_raw);
    if estimated_order_volume <= 0.0 {
        bail!("EX_ANTE_COST_UNAVAILABLE: estimated order volume is not positive");
    }

    Ok(TradeCalculation {
        sizing_price_basis: sizing_price.basis,
        sizing_price_value: sizing_price.price,
        estimate_price_basis: estimate_price.basis,
        estimate_price_value: estimate_price.price,
        number_of_shares,
        number_of_shares_str,
        is_whole_position_sold,
        estimated_order_volume_raw,
        estimated_order_volume,
    })
}

fn pricing_basis_display_name(basis: &str) -> &'static str {
    match basis {
        "ask_price" => "askPrice",
        "bid_price" => "bidPrice",
        "limit_price" => "limitPrice",
        "stop_price" => "stopPrice",
        _ => "price",
    }
}

fn submit_order(
    prepared: &PreparedTrade,
    confirmation_id: &str,
    confirmation_expires_at_epoch: i64,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    crate::channel::require_current_channel(prepared.env)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, prepared.env, &env_cfg, dpop_options)?;
    let mut session = loaded.session;
    let access_context = loaded.access_context;

    let intent_hash = trade_intent_hash(TradeIntentHashInput {
        env: prepared.env,
        account_id: &prepared.account_id,
        portfolio_id: &prepared.portfolio_id,
        side: side_label(prepared.intent.side),
        order_type: &prepared.intent.order_type,
        isin: &prepared.intent.isin,
        amount: prepared.intent.amount_str.as_deref(),
        shares_input: prepared.intent.shares_str.as_deref(),
        limit_price: prepared.intent.limit_price_str.as_deref(),
        stop_price: prepared.intent.stop_price_str.as_deref(),
        shares: &prepared.number_of_shares_str,
        estimated_order_volume: &canonical_decimal_from_f64(prepared.estimated_order_volume),
        venue: &prepared.confirmation_fields.venue,
    });

    let now_epoch = current_epoch_seconds();
    if let Some(existing_submission) =
        load_recent_submitted_attempt(prepared.env, &intent_hash, now_epoch)?
    {
        return Ok(json!({
            "submitted": true,
            "idempotency_key": existing_submission.idempotency_key,
            "idempotency_reused": true,
            "order_id": existing_submission.order_id,
            "is_marketable": Value::Null,
        }));
    }

    let attempt = start_or_reuse_attempt(prepared.env, &intent_hash, now_epoch)?;

    mark_trade_attempt_in_flight(
        prepared.env,
        &attempt.idempotency_key,
        current_epoch_seconds(),
    )
    .context("Failed to persist trade attempt in-flight state")?;

    let place_order_variables = trade_place_order_variables(PlaceOrderFields {
        side: prepared.intent.side,
        portfolio_id: &prepared.portfolio_id,
        isin: &prepared.intent.isin,
        number_of_shares: prepared.number_of_shares,
        currency: &prepared.quote_currency,
        venue: &prepared.confirmation_fields.venue,
        limit_price: prepared.intent.limit_price,
        stop_price: prepared.intent.stop_price,
        appropriateness_id: prepared.appropriateness_id_for_order.as_deref(),
        acknowledged_warning_version: prepared.warning_version_for_order.as_deref(),
        fill_forecast_id: None,
        displayed_fill_probability: None,
    })?;

    let submit_response = execute_with_refresh_retry(
        session_manager,
        prepared.env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql_with_headers(
                &env_cfg.graphql_url,
                token,
                TRADE_PLACE_ORDER_MUTATION,
                &place_order_variables,
                Some("placeOrder"),
                access_context,
                &[("X-SC-Idempotency-Id", attempt.idempotency_key.as_str())],
                dpop_options,
            )
        },
    );

    match submit_response {
        Ok(response) => {
            let order = parse_place_order_result(&response)?;
            mark_trade_attempt_submitted(
                prepared.env,
                &attempt.idempotency_key,
                &order.order_id,
                current_epoch_seconds(),
            )
            .context("Failed to persist trade attempt submitted state")?;
            Ok(json!({
                "submitted": true,
                "idempotency_key": attempt.idempotency_key,
                "idempotency_reused": attempt.reused,
                "order_id": order.order_id,
                "is_marketable": order.is_marketable,
            }))
        }
        Err(err) => {
            let error_message = err.to_string();
            let _ = mark_trade_attempt_failed(
                prepared.env,
                &attempt.idempotency_key,
                &error_message,
                current_epoch_seconds(),
            );
            if should_passthrough_order_submission_error(&error_message) {
                return Err(err);
            }
            Err(anyhow!(format_order_submission_failed_message(
                &error_message,
                &attempt.idempotency_key,
                confirmation_id,
                confirmation_expires_at_epoch,
            )))
        }
    }
}

fn should_passthrough_order_submission_error(error_message: &str) -> bool {
    error_message.contains(LOCAL_READ_ONLY_ERROR_PREFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrderSubmissionFailureKind {
    BackendResponse,
    AmbiguousOutcome,
}

fn classify_order_submission_failure(error_message: &str) -> OrderSubmissionFailureKind {
    let lower = error_message.to_lowercase();
    if parse_graphql_http_error_status(error_message).is_some() {
        return OrderSubmissionFailureKind::AmbiguousOutcome;
    }

    if error_message.contains("GraphQL returned errors") {
        return OrderSubmissionFailureKind::BackendResponse;
    }

    if error_message.contains("Failed to call GraphQL endpoint")
        || error_message.contains("Failed to parse GraphQL JSON response")
        || error_message.contains("GraphQL response missing data")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection")
    {
        return OrderSubmissionFailureKind::AmbiguousOutcome;
    }

    OrderSubmissionFailureKind::AmbiguousOutcome
}

fn parse_graphql_http_error_status(error_message: &str) -> Option<u16> {
    error_message
        .strip_prefix("GraphQL HTTP error ")
        .and_then(|suffix| suffix.split_whitespace().next())
        .and_then(|status| status.parse::<u16>().ok())
}

fn format_order_submission_failed_message(
    error_message: &str,
    idempotency_key: &str,
    confirmation_id: &str,
    confirmation_expires_at_epoch: i64,
) -> String {
    let error_message = error_message.trim_end_matches(['.', '!', '?']);
    let retry_guidance = format!(
        "confirmation id {confirmation_id} remains valid until epoch {confirmation_expires_at_epoch}; the same idempotency key {idempotency_key} will be reused"
    );
    match classify_order_submission_failure(error_message) {
        OrderSubmissionFailureKind::BackendResponse => format!(
            "ORDER_SUBMISSION_FAILED: {error_message}. The backend responded with an error, so treat this submit as failed. Do not blindly retry. If you intentionally retry this exact phase-2 submit, reuse the same confirmation while {retry_guidance}. If phase 2 then fails with a confirmation error, rerun phase 1."
        ),
        OrderSubmissionFailureKind::AmbiguousOutcome => format!(
            "ORDER_SUBMISSION_FAILED: {error_message}. The submit outcome may be ambiguous. Check transactions or order status first. If the order is not present, retry this exact phase-2 submit while {retry_guidance}."
        ),
    }
}

fn required_flag_value<'a>(value: Option<&'a str>, flag: &str) -> Result<&'a str> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("CONFIRMATION_REQUIRED: missing required flag {}", flag))
}

fn optional_trimmed_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn side_label(side: TradeSide) -> &'static str {
    match side {
        TradeSide::Buy => ORDER_SIDE_BUY,
        TradeSide::Sell => ORDER_SIDE_SELL,
    }
}

fn display_venue_label(selected_venue: &str) -> String {
    display_venue_label_from_module(selected_venue)
}

fn resolve_sell_trade_shares(
    requested_shares: f64,
    requested_shares_str: Option<&str>,
    selected_venue: &str,
    selected_venue_sellable: Option<f64>,
) -> Result<(f64, String, bool)> {
    let sellable = selected_venue_sellable.ok_or_else(|| {
        anyhow!(
            "Trade response invalid: missing sellable quantity for selected venue '{}'",
            selected_venue
        )
    })?;
    if requested_shares > sellable + decimal_tolerance(sellable) {
        bail!(
            "Trade input invalid: shares {} exceed sellable quantity {} on venue '{}'",
            requested_shares_str.unwrap_or(VALUE_NA),
            canonical_decimal_from_f64(sellable),
            selected_venue
        );
    }
    let whole = decimal_equal(requested_shares, sellable);
    Ok((
        requested_shares,
        requested_shares_str
            .map(ToString::to_string)
            .unwrap_or_else(|| canonical_decimal_from_f64(requested_shares)),
        whole,
    ))
}

fn decimal_tolerance(reference: f64) -> f64 {
    let scale = reference.abs().max(1.0);
    scale * 1e-8
}

fn decimal_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= decimal_tolerance(left.max(right))
}

fn validate_order_price_flags(
    order_type: crate::cli::BrokerTradeOrderType,
    limit_price_raw: Option<&str>,
    stop_price_raw: Option<&str>,
) -> Result<ValidatedOrderPrices> {
    let limit_price = optional_trimmed_string(limit_price_raw);
    let stop_price = optional_trimmed_string(stop_price_raw);

    if limit_price.is_some() && stop_price.is_some() {
        bail!("Trade input invalid: --limit-price and --stop-price cannot be used together");
    }

    match order_type {
        crate::cli::BrokerTradeOrderType::Market => {
            if limit_price.is_some() || stop_price.is_some() {
                bail!(
                    "Trade input invalid: --order-type market does not allow --limit-price or --stop-price"
                );
            }
        }
        crate::cli::BrokerTradeOrderType::Limit => {
            if limit_price.is_none() {
                bail!("Trade input invalid: --limit-price is required for --order-type limit");
            }
            if stop_price.is_some() {
                bail!("Trade input invalid: --order-type limit does not allow --stop-price");
            }
        }
        crate::cli::BrokerTradeOrderType::Stop => {
            if stop_price.is_none() {
                bail!("Trade input invalid: --stop-price is required for --order-type stop");
            }
            if limit_price.is_some() {
                bail!("Trade input invalid: --order-type stop does not allow --limit-price");
            }
        }
    }

    let limit_price_value = match limit_price.as_deref() {
        Some(value) => Some(parse_positive_decimal_from_str(value, "limit_price")?.0),
        None => None,
    };
    let stop_price_value = match stop_price.as_deref() {
        Some(value) => Some(parse_positive_decimal_from_str(value, "stop_price")?.0),
        None => None,
    };

    Ok(ValidatedOrderPrices {
        limit_price,
        stop_price,
        limit_price_value,
        stop_price_value,
    })
}

fn order_type_label(order_type: crate::cli::BrokerTradeOrderType) -> &'static str {
    match order_type {
        crate::cli::BrokerTradeOrderType::Market => ORDER_TYPE_MARKET,
        crate::cli::BrokerTradeOrderType::Limit => ORDER_TYPE_LIMIT,
        crate::cli::BrokerTradeOrderType::Stop => ORDER_TYPE_STOP,
    }
}

fn required_input_value<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "Trade input invalid: field '{}' must be a non-empty string",
                field
            )
        })
}

fn canonical_isin(value: &str, field: &str) -> Result<String> {
    let normalized = value.trim().to_uppercase();
    if normalized.is_empty() {
        return Err(anyhow!(
            "Trade input invalid: field '{}' must be a non-empty string",
            field
        ));
    }
    Ok(normalized)
}

fn canonical_required_upper(value: &str, field: &str) -> Result<String> {
    let normalized = value.trim().to_uppercase();
    if normalized.is_empty() {
        return Err(anyhow!(
            "Trade input invalid: field '{}' must be a non-empty string",
            field
        ));
    }
    Ok(normalized)
}

fn parse_positive_decimal_from_str(raw: &str, field: &str) -> Result<(f64, String)> {
    let normalized = normalize_decimal_str(raw);
    let parsed = normalized.parse::<f64>().map_err(|_| {
        anyhow!(
            "Trade input invalid: field '{}' must be a positive decimal",
            field
        )
    })?;
    if !parsed.is_finite() || parsed <= 0.0 {
        bail!(
            "Trade input invalid: field '{}' must be a positive decimal",
            field
        );
    }
    Ok((parsed, normalized))
}

fn normalize_decimal_str(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "0".to_string();
    }
    if trimmed.eq_ignore_ascii_case(VALUE_NA) {
        return VALUE_NA.to_string();
    }

    let mut chars = trimmed.chars();
    let negative = matches!(chars.next(), Some('-'));
    let unsigned = if negative || trimmed.starts_with('+') {
        &trimmed[1..]
    } else {
        trimmed
    };

    if unsigned.is_empty() {
        return "0".to_string();
    }

    let parts = unsigned.split('.').collect::<Vec<_>>();
    if parts.len() > 2
        || !parts
            .iter()
            .all(|segment| segment.chars().all(|ch| ch.is_ascii_digit()))
    {
        return trimmed.to_string();
    }

    let integer_raw = parts[0].trim_start_matches('0');
    let integer = if integer_raw.is_empty() {
        "0"
    } else {
        integer_raw
    };
    let fraction = if parts.len() == 2 {
        parts[1].trim_end_matches('0')
    } else {
        ""
    };

    let mut normalized = if fraction.is_empty() {
        integer.to_string()
    } else {
        format!("{integer}.{fraction}")
    };

    if negative && normalized != "0" {
        normalized = format!("-{normalized}");
    }

    normalized
}

pub(crate) fn canonical_decimal_from_f64(value: f64) -> String {
    normalize_decimal_str(&format!("{:.8}", value))
}

fn canonical_cost_value(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return VALUE_NA.to_string();
    }
    if trimmed.eq_ignore_ascii_case(VALUE_NA) {
        return VALUE_NA.to_string();
    }
    normalize_decimal_str(trimmed)
}

fn canonical_cost_from_json(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => canonical_cost_value(s),
        Some(Value::Number(n)) => canonical_cost_value(&n.to_string()),
        Some(Value::Null) | None => VALUE_NA.to_string(),
        Some(other) => canonical_cost_value(&other.to_string()),
    }
}

struct IntentChecksumInput<'a> {
    env: TargetEnv,
    account_id: &'a str,
    portfolio_id: &'a str,
    side: &'a str,
    order_type: &'a str,
    isin: &'a str,
    amount: Option<&'a str>,
    shares: Option<&'a str>,
    limit_price: Option<&'a str>,
    stop_price: Option<&'a str>,
    venue_override: Option<&'a str>,
}

fn intent_checksum_payload(input: IntentChecksumInput<'_>) -> Value {
    json!({
        "version": "intent_v1",
        "env": input.env.as_str(),
        "account_id": input.account_id,
        "portfolio_id": input.portfolio_id,
        "side": input.side,
        "order_type": input.order_type,
        "isin": input.isin,
        "amount": input.amount,
        "shares": input.shares,
        "limit_price": input.limit_price,
        "stop_price": input.stop_price,
        "venue_override": input.venue_override,
    })
}

fn build_phase2_command_template(
    confirmation_id: &str,
    phase1_input: &ConfirmationPhase1Input,
    requires_accept_unsuitable: bool,
    json_mode: bool,
) -> String {
    build_phase2_command_template_from_module(
        confirmation_id,
        phase1_input,
        requires_accept_unsuitable,
        json_mode,
    )
}

fn build_phase1_command_template(
    phase1_input: &ConfirmationPhase1Input,
    json_mode: bool,
) -> String {
    build_phase1_command_template_from_module(phase1_input, json_mode)
}

fn build_result_payload(prepared: &PreparedTrade, order_submission: Option<Value>) -> Value {
    build_result_payload_from_module(prepared, order_submission)
}

fn build_warning_payload(decision: &TradeComplianceDecision) -> Value {
    if matches!(decision.warning.kind, crate::trade::TradeWarningKind::None) {
        return Value::Null;
    }

    json!({
        "kind": decision.warning.kind.as_str(),
        "title": decision.warning.title,
        "body": decision.warning.body,
        "locale": decision.warning.locale,
        "version": decision.warning.version_for_order,
        "acknowledgement_text": decision.warning.acknowledgement_text,
    })
}

fn build_suitability_payload(decision: &TradeComplianceDecision) -> Value {
    json!({
        "source": decision.source_kind.as_str(),
        "type": decision.suitability_type.map(|value| value.as_str()),
        "status": decision.status.as_str(),
        "action_when_unsuitable": decision.action_when_unsuitable.map(|value| value.as_str()),
        "questionnaire_required": decision.questionnaire_required,
        "questionnaire_reason": decision.questionnaire_reason,
        "requires_accept_unsuitable": decision.requires_accept_unsuitable,
        "accept_flag": decision.requires_accept_unsuitable.then_some("--accept-unsuitable"),
    })
}

fn questionnaire_name_for_error(decision: &TradeComplianceDecision) -> &'static str {
    match decision.source_kind {
        crate::trade::TradeComplianceSourceKind::LegacyAppropriatenessFallback => "appropriateness",
        _ => decision
            .suitability_type
            .map(|kind| kind.as_str())
            .unwrap_or("UNKNOWN"),
    }
}

fn phase1_requires_accept_unsuitable(prepared: &PreparedTrade) -> bool {
    prepared.intent.side.is_buy() && prepared.compliance_decision.requires_accept_unsuitable
}

fn phase1_required_json_paths(side: TradeSide) -> Vec<&'static str> {
    phase1_required_json_paths_from_module(side)
}

fn presentation_section_order_keys(side: TradeSide) -> Vec<&'static str> {
    presentation_section_order_keys_from_module(side)
}

fn build_phase1_presentation(result: &Value, confirmation: &Value) -> Result<Value> {
    build_phase1_presentation_from_module(result, confirmation)
}

fn presentation_required_paths_from_presentation(presentation: &Value) -> Result<Value> {
    match presentation.get("required_leaf_paths") {
        Some(Value::Array(values)) if values.iter().all(Value::is_string) => {
            Ok(Value::Array(values.clone()))
        }
        Some(Value::Array(_)) => {
            bail!("presentation.required_leaf_paths must contain only string paths")
        }
        Some(_) => bail!("presentation.required_leaf_paths must be an array"),
        None => bail!("presentation.required_leaf_paths is missing from presentation"),
    }
}

pub(crate) fn render_trade_buy_text(payload: &Value) -> Vec<String> {
    render_trade_buy_text_from_module(payload)
}

pub(crate) fn render_trade_sell_text(payload: &Value) -> Vec<String> {
    render_trade_sell_text_from_module(payload)
}

pub(crate) fn render_trade_cancel_text(payload: &Value) -> Vec<String> {
    let order_id = payload
        .pointer("/result/order_id")
        .and_then(Value::as_str)
        .unwrap_or(VALUE_NA);

    vec![
        "Cancellation requested.".to_string(),
        format!("order_id: {order_id}"),
    ]
}

fn confirmation_id_from_nonce_and_checksum(nonce: &str, checksum: &str) -> String {
    let digest = sha256_hex(format!("{nonce}:{checksum}").as_bytes());
    format!("scb1_{}", &digest[..26])
}

struct TradeIntentHashInput<'a> {
    env: TargetEnv,
    account_id: &'a str,
    portfolio_id: &'a str,
    side: &'a str,
    order_type: &'a str,
    isin: &'a str,
    amount: Option<&'a str>,
    shares_input: Option<&'a str>,
    limit_price: Option<&'a str>,
    stop_price: Option<&'a str>,
    shares: &'a str,
    estimated_order_volume: &'a str,
    venue: &'a str,
}

fn trade_intent_hash(input: TradeIntentHashInput<'_>) -> String {
    let payload = json!({
        "env": input.env.as_str(),
        "account_id": input.account_id,
        "portfolio_id": input.portfolio_id,
        "side": input.side,
        "order_type": input.order_type,
        "isin": input.isin,
        "amount": input.amount,
        "shares_input": input.shares_input,
        "limit_price": input.limit_price,
        "stop_price": input.stop_price,
        "shares": input.shares,
        "estimated_order_volume": input.estimated_order_volume,
        "venue": input.venue,
    });
    sha256_hex(payload.to_string().as_bytes())
}

fn current_epoch_seconds() -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{LoginSource, Session, StoredSession};
    use crate::trade_presentation::{
        BUY_EX_ANTE_COSTS_NOTICE, BUY_XETRA_MARKET_DATA_NOTICE, CLIENT_DOCUMENTS_URL,
        PRESENTATION_MAPPING_INCOMPLETE, build_buy_regulatory_disclosures,
        build_sell_regulatory_disclosures, phase1_presentation_sections,
    };
    use mockito::{Matcher, Server};

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

    #[test]
    fn order_submission_failed_message_avoids_double_periods() {
        let message = format_order_submission_failed_message(
            "GraphQL returned errors for placeOrder (code: UNEXPECTED).",
            "4251d813-d33b-49c9-9a9c-f93fca0b501b",
            "scb1_test",
            1_777_777_777,
        );

        assert!(!message.contains(".."));
        assert!(message.starts_with(
            "ORDER_SUBMISSION_FAILED: GraphQL returned errors for placeOrder (code: UNEXPECTED)."
        ));
    }

    #[test]
    fn local_read_only_order_submission_errors_are_not_wrapped() {
        assert!(should_passthrough_order_submission_error(
            "ORDER_SUBMISSION_FAILED: ignored prefix LOCAL_READ_ONLY: local read-only mode blocks write operation 'placeOrder'."
        ));
        assert!(should_passthrough_order_submission_error(
            "LOCAL_READ_ONLY: local read-only mode blocks write operation 'placeOrder'."
        ));
        assert!(!should_passthrough_order_submission_error("timeout"));
    }

    #[test]
    fn order_submission_failed_message_treats_graphql_errors_as_backend_responses() {
        let message = format_order_submission_failed_message(
            "GraphQL returned errors for placeOrder (code: UNEXPECTED).",
            "4251d813-d33b-49c9-9a9c-f93fca0b501b",
            "scb1_test",
            1_777_777_777,
        );
        assert!(message.contains("The backend responded with an error"));
        assert!(message.contains("Do not blindly retry"));
        assert!(message.contains("If phase 2 then fails with a confirmation error, rerun phase 1"));
        assert!(message.contains("confirmation id scb1_test remains valid until epoch 1777777777"));
    }

    #[test]
    fn order_submission_failed_message_treats_transport_timeouts_as_ambiguous() {
        let message = format_order_submission_failed_message(
            "Failed to call GraphQL endpoint: operation timed out.",
            "4251d813-d33b-49c9-9a9c-f93fca0b501b",
            "scb1_test",
            1_777_777_777,
        );
        assert!(message.contains("The submit outcome may be ambiguous"));
        assert!(message.contains("Check transactions or order status first"));
    }

    #[test]
    fn classify_order_submission_failure_detects_backend_response_errors() {
        assert_eq!(
            classify_order_submission_failure(
                "GraphQL returned errors for placeOrder (code: UNEXPECTED)"
            ),
            OrderSubmissionFailureKind::BackendResponse
        );
    }

    #[test]
    fn classify_order_submission_failure_detects_ambiguous_outcomes() {
        assert_eq!(
            classify_order_submission_failure("GraphQL HTTP error 400 during placeOrder"),
            OrderSubmissionFailureKind::AmbiguousOutcome
        );
        assert_eq!(
            classify_order_submission_failure("GraphQL HTTP error 503 during placeOrder"),
            OrderSubmissionFailureKind::AmbiguousOutcome
        );
        assert_eq!(
            classify_order_submission_failure("GraphQL HTTP error during placeOrder"),
            OrderSubmissionFailureKind::AmbiguousOutcome
        );
        assert_eq!(
            classify_order_submission_failure(
                "Failed to call GraphQL endpoint: operation timed out"
            ),
            OrderSubmissionFailureKind::AmbiguousOutcome
        );
        assert_eq!(
            classify_order_submission_failure("Failed to parse GraphQL JSON response"),
            OrderSubmissionFailureKind::AmbiguousOutcome
        );
        assert_eq!(
            classify_order_submission_failure("GraphQL response missing data"),
            OrderSubmissionFailureKind::AmbiguousOutcome
        );
        assert_eq!(
            classify_order_submission_failure("connection reset by peer"),
            OrderSubmissionFailureKind::AmbiguousOutcome
        );
    }

    fn sample_session() -> Session {
        Session {
            access_token: "test-token".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        }
    }

    fn sample_stored_session(env: crate::config::TargetEnv) -> StoredSession {
        StoredSession {
            env,
            session: sample_session(),
            dpop_jwk_thumbprint: Some(current_runtime_dpop_thumbprint()),
            mode: None,
        }
    }

    fn sample_config() -> AppConfig {
        AppConfig {
            auth: crate::config::RuntimeAuthConfig {
                session_backend: crate::config::SessionBackendPreference::File,
                signing_key_backend: crate::config::DpopKeyBackend::File,
                pkcs11: None,
            },
            trade_controls: None,
        }
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

    fn current_runtime_dpop_thumbprint() -> String {
        crate::dpop::DpopKeyMaterial::load_existing_for_options(
            &crate::channel::current_dpop_runtime_options(&sample_config()),
        )
        .expect("load runtime dpop key")
        .jwk_thumbprint()
        .expect("runtime dpop thumbprint")
    }

    fn sample_result_payload() -> Value {
        json!({
            "intent": {
                "side": "buy",
                "isin": "DE0007100000",
                "amount": "500",
                "order_type": "market",
                "venue_override": Value::Null,
                "locale": "en_DE"
            },
            "market_quote": {
                "ask_price": "50.5000",
                "bid_price": "50.4000",
                "mid_price": "50.4561",
                "currency": "EUR",
                "is_outdated": false,
                "timestamp_utc": "2026-03-10T19:25:29.000Z"
            },
            "calculation": {
                "sizing_price_basis": "ask_price",
                "sizing_price": "50.5000",
                "estimate_price_basis": "ask_price",
                "estimate_price": "50.5000",
                "shares": "9",
                "estimated_order_volume_raw": "454.5000",
                "estimated_order_volume": "454.5000"
            },
            "tradability": {
                "status": "TRADABLE_WITHOUT_APPROPRIATENESS",
                "selected_venue": "SEIX",
                "selected_venue_label": "European Investor Exchange (EIX)",
                "selected_venue_status": "TRADABLE_WITHOUT_APPROPRIATENESS",
                "selected_venue_unavailability_reason": Value::Null,
                "requires_appropriateness": false,
                "tradable": true
            },
            "suitability": {
                "source": "not_required",
                "type": Value::Null,
                "status": "NOT_REQUIRED",
                "action_when_unsuitable": Value::Null,
                "questionnaire_required": false,
                "questionnaire_reason": Value::Null,
                "requires_accept_unsuitable": false,
                "accept_flag": Value::Null
            },
            "warning": Value::Null,
            "price_warnings": {
                "items": []
            },
            "ex_ante_costs": {
                "id": "cost-id",
                "entryCosts": {
                    "serviceCosts": {"amount": 0.12, "percentage": 0.00026},
                    "productCosts": {"amount": 0.34, "percentage": 0.00075},
                    "total": {"amount": 0.46, "percentage": 0.00101}
                },
                "ongoingCosts": {
                    "serviceCosts": {"amount": 0.05, "percentage": 0.00011},
                    "productCosts": {"amount": 0.07, "percentage": 0.00015},
                    "total": {"amount": 0.12, "percentage": 0.00026}
                },
                "exitCosts": {
                    "serviceCosts": {"amount": 0.08, "percentage": 0.00018},
                    "productCosts": {"amount": 0.09, "percentage": 0.00019},
                    "total": {"amount": 0.17, "percentage": 0.00037}
                },
                "effectOnReturn": {
                    "initialYearCosts": {"amount": 0.46, "percentage": 0.00101},
                    "followingYearsCosts": {"amount": 0.12, "percentage": 0.00026},
                    "finalYearCosts": {"amount": 0.17, "percentage": 0.00037}
                },
                "incidentalCosts": {"amount": 0.03, "percentage": 0.00007},
                "fiveYearsCosts": {"amount": 2.50, "percentage": 0.00550}
            },
            "regulatory_disclosures": {
                "market_data_notice": Value::Null,
                "execution_instruction": "I instruct Scalable Capital to place this order for execution on European Investor Exchange (EIX) valid for up to 360 days.",
                "ex_ante_costs_notice": BUY_EX_ANTE_COSTS_NOTICE
            },
            "document_links": {
                "client_documents": {
                    "label": "Client documents",
                    "url": CLIENT_DOCUMENTS_URL
                },
                "primary_kid": {
                    "label": "Key information document (KID)",
                    "url": "https://example.test/primary-kid.pdf"
                },
                "secondary_kid": {
                    "label": "Key information document (KID)",
                    "url": "https://example.test/en-kid.pdf"
                }
            }
        })
    }

    fn sample_sell_result_payload() -> Value {
        json!({
            "intent": {
                "side": "sell",
                "isin": "DE0007100000",
                "amount": Value::Null,
                "shares": "9",
                "order_type": "market",
                "venue_override": Value::Null,
                "locale": "en_DE"
            },
            "market_quote": {
                "ask_price": "50.5000",
                "bid_price": "50.4000",
                "mid_price": "50.4561",
                "currency": "EUR",
                "is_outdated": false,
                "timestamp_utc": "2026-03-10T19:25:29.000Z"
            },
            "calculation": {
                "sizing_price_basis": "bid_price",
                "sizing_price": "50.4000",
                "estimate_price_basis": "bid_price",
                "estimate_price": "50.4000",
                "shares": "9",
                "estimated_order_volume_raw": "453.6000",
                "estimated_order_volume": "453.6000",
                "is_whole_position_sold": false
            },
            "tradability": {
                "status": "TRADABLE_WITHOUT_APPROPRIATENESS",
                "selected_venue": "GETTEX",
                "selected_venue_label": "Börse München (gettex)",
                "selected_venue_status": "TRADABLE_WITHOUT_APPROPRIATENESS",
                "selected_venue_unavailability_reason": Value::Null,
                "requires_appropriateness": false,
                "tradable": true,
                "selected_venue_sellable": 9.0
            },
            "suitability": {
                "source": "not_required",
                "type": Value::Null,
                "status": "NOT_REQUIRED",
                "action_when_unsuitable": Value::Null,
                "questionnaire_required": false,
                "questionnaire_reason": Value::Null,
                "requires_accept_unsuitable": false,
                "accept_flag": Value::Null
            },
            "warning": Value::Null,
            "price_warnings": {
                "items": []
            },
            "ex_ante_costs": {
                "id": "cost-id",
                "entryCosts": {
                    "serviceCosts": {"amount": 0.12, "percentage": 0.00026},
                    "productCosts": {"amount": 0.34, "percentage": 0.00075},
                    "total": {"amount": 0.46, "percentage": 0.00101}
                },
                "ongoingCosts": {
                    "serviceCosts": {"amount": 0.05, "percentage": 0.00011},
                    "productCosts": {"amount": 0.07, "percentage": 0.00015},
                    "total": {"amount": 0.12, "percentage": 0.00026}
                },
                "exitCosts": {
                    "serviceCosts": {"amount": 0.08, "percentage": 0.00018},
                    "productCosts": {"amount": 0.09, "percentage": 0.00019},
                    "total": {"amount": 0.17, "percentage": 0.00037}
                },
                "effectOnReturn": {
                    "initialYearCosts": {"amount": 0.46, "percentage": 0.00101},
                    "followingYearsCosts": {"amount": 0.12, "percentage": 0.00026},
                    "finalYearCosts": {"amount": 0.17, "percentage": 0.00037}
                },
                "incidentalCosts": {"amount": 0.03, "percentage": 0.00007},
                "fiveYearsCosts": {"amount": 2.50, "percentage": 0.00550}
            },
            "regulatory_disclosures": {
                "execution_instruction": "I instruct Scalable Capital to place this order for execution on Börse München (gettex) valid for up to 360 days."
            },
            "document_links": {
                "client_documents": {
                    "label": "Client documents",
                    "url": CLIENT_DOCUMENTS_URL
                },
                "primary_kid": {
                    "label": "Key information document (KID)",
                    "url": "https://example.test/primary-kid.pdf"
                },
                "secondary_kid": Value::Null
            },
            "order_submission": {
                "submitted": false,
                "reason": "phase_1_preview_only"
            },
            "pre_trade_checks_passed": true
        })
    }

    fn sample_confirmation_payload() -> Value {
        json!({
            "id": "scb1_test",
            "expires_at_epoch": 1_777_777_777
        })
    }

    fn find_field<'a>(presentation: &'a Value, path: &str) -> Option<&'a Value> {
        presentation
            .get("sections")?
            .as_object()?
            .values()
            .find_map(|section| {
                section
                    .get("fields")
                    .and_then(Value::as_array)
                    .and_then(|fields| {
                        fields
                            .iter()
                            .find(|field| field.get("path").and_then(Value::as_str) == Some(path))
                    })
            })
    }

    #[test]
    fn phase1_presentation_has_order_and_schema() {
        let presentation =
            build_phase1_presentation(&sample_result_payload(), &sample_confirmation_payload())
                .expect("presentation should build");

        assert_eq!(
            presentation.get("format").and_then(Value::as_str),
            Some(PHASE1_PRESENTATION_FORMAT)
        );
        assert_eq!(
            presentation
                .get("section_order")
                .and_then(Value::as_array)
                .map(|v| v.len()),
            Some(phase1_presentation_sections(TradeSide::Buy).len())
        );
        let field = find_field(&presentation, "/result/intent/isin").expect("field should exist");
        assert_eq!(field.get("label").and_then(Value::as_str), Some("ISIN"));
        assert_eq!(
            field.get("value").and_then(Value::as_str),
            Some("DE0007100000")
        );
        assert_eq!(
            field.get("value_type").and_then(Value::as_str),
            Some("string")
        );

        let required_leaf_paths = presentation
            .get("required_leaf_paths")
            .and_then(Value::as_array)
            .expect("required leaf paths");
        assert!(
            required_leaf_paths.iter().any(|item| item.as_str()
                == Some("/result/ex_ante_costs/entryCosts/serviceCosts/amount"))
        );
        assert!(
            required_leaf_paths.iter().any(|item| item.as_str()
                == Some("/result/ex_ante_costs/entryCosts/productCosts/amount"))
        );
    }

    #[test]
    fn phase1_presentation_fails_on_missing_required_path() {
        let mut result = sample_result_payload();
        result
            .as_object_mut()
            .expect("object")
            .remove("tradability");

        let err = build_phase1_presentation(&result, &sample_confirmation_payload())
            .expect_err("should fail");
        assert!(err.to_string().contains(PRESENTATION_MAPPING_INCOMPLETE));
    }

    #[test]
    fn phase1_presentation_fails_on_missing_ex_ante_required_leaf() {
        let mut result = sample_result_payload();
        let ex_ante_costs = result
            .as_object_mut()
            .expect("object")
            .get_mut("ex_ante_costs")
            .expect("ex ante costs")
            .as_object_mut()
            .expect("ex ante costs object");
        let ongoing_costs = ex_ante_costs
            .get_mut("ongoingCosts")
            .expect("ongoing costs")
            .as_object_mut()
            .expect("ongoing costs object");
        let service_costs = ongoing_costs
            .get_mut("serviceCosts")
            .expect("service costs")
            .as_object_mut()
            .expect("service costs object");
        service_costs.remove("amount");

        let err = build_phase1_presentation(&result, &sample_confirmation_payload())
            .expect_err("should fail");
        assert!(
            err.to_string()
                .contains("/result/ex_ante_costs/ongoingCosts/serviceCosts/amount")
        );
    }

    #[test]
    fn phase1_presentation_allows_null_entry_costs_in_result_payload() {
        let mut result = sample_result_payload();
        result["ex_ante_costs"]["entryCosts"] = Value::Null;

        let presentation =
            build_phase1_presentation(&result, &sample_confirmation_payload()).expect("build");
        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/entryCosts/serviceCosts/amount",
            )
            .is_none()
        );

        let required_leaf_paths = presentation
            .get("required_leaf_paths")
            .and_then(Value::as_array)
            .expect("required leaf paths");
        assert!(
            !required_leaf_paths.iter().any(|item| item.as_str()
                == Some("/result/ex_ante_costs/entryCosts/serviceCosts/amount"))
        );
    }

    #[test]
    fn phase1_presentation_omits_null_effect_on_return_sell_fields_in_result_payload() {
        let mut result = sample_sell_result_payload();
        result["ex_ante_costs"]["effectOnReturn"]["initialYearCosts"] = Value::Null;
        result["ex_ante_costs"]["effectOnReturn"]["followingYearsCosts"] = Value::Null;

        let presentation =
            build_phase1_presentation(&result, &sample_confirmation_payload()).expect("build");
        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/effectOnReturn/initialYearCosts/amount",
            )
            .is_none()
        );
        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/effectOnReturn/followingYearsCosts/amount",
            )
            .is_none()
        );
        assert!(
            find_field(
                &presentation,
                "/result/ex_ante_costs/effectOnReturn/finalYearCosts/amount",
            )
            .is_some()
        );

        let required_leaf_paths = presentation
            .get("required_leaf_paths")
            .and_then(Value::as_array)
            .expect("required leaf paths");
        assert!(!required_leaf_paths.iter().any(|item| item.as_str()
            == Some("/result/ex_ante_costs/effectOnReturn/initialYearCosts/amount")));
        assert!(!required_leaf_paths.iter().any(|item| item.as_str()
            == Some("/result/ex_ante_costs/effectOnReturn/followingYearsCosts/amount")));
        assert!(required_leaf_paths.iter().any(|item| item.as_str()
            == Some("/result/ex_ante_costs/effectOnReturn/finalYearCosts/amount")));
    }

    #[test]
    fn presentation_required_paths_from_presentation_fails_closed_on_missing_field() {
        let err = presentation_required_paths_from_presentation(&json!({
            "format": PHASE1_PRESENTATION_FORMAT
        }))
        .expect_err("missing required leaf paths should fail");

        assert_eq!(
            err.to_string(),
            "presentation.required_leaf_paths is missing from presentation"
        );
    }

    #[test]
    fn presentation_required_paths_from_presentation_fails_closed_on_non_array() {
        let err = presentation_required_paths_from_presentation(&json!({
            "required_leaf_paths": "nope"
        }))
        .expect_err("non-array required leaf paths should fail");

        assert_eq!(
            err.to_string(),
            "presentation.required_leaf_paths must be an array"
        );
    }

    #[test]
    fn presentation_required_paths_from_presentation_fails_closed_on_non_string_entries() {
        let err = presentation_required_paths_from_presentation(&json!({
            "required_leaf_paths": ["/result/intent/isin", 1]
        }))
        .expect_err("non-string required leaf paths should fail");

        assert_eq!(
            err.to_string(),
            "presentation.required_leaf_paths must contain only string paths"
        );
    }

    #[test]
    fn phase1_presentation_renders_nullable_warning_fields_as_null() {
        let mut result = sample_result_payload();
        result["warning"] = Value::Null;

        let presentation =
            build_phase1_presentation(&result, &sample_confirmation_payload()).expect("build");
        let field = find_field(&presentation, "/result/warning/version")
            .expect("warning field should exist");
        assert_eq!(field.get("value"), Some(&Value::Null));
        assert_eq!(
            field.get("value_type").and_then(Value::as_str),
            Some("null")
        );
        let required_leaf_paths = presentation
            .get("required_leaf_paths")
            .and_then(Value::as_array)
            .expect("required leaf paths");
        assert!(
            required_leaf_paths
                .iter()
                .any(|item| item.as_str() == Some("/result/warning/version"))
        );
    }

    #[test]
    fn phase1_presentation_keeps_intent_locale_when_warning_locale_uses_wire_format() {
        let mut result = sample_result_payload();
        result["warning"]["locale"] = json!("en-DE");

        let presentation =
            build_phase1_presentation(&result, &sample_confirmation_payload()).expect("build");
        let intent_locale = find_field(&presentation, "/result/intent/locale")
            .expect("intent locale field should exist");
        let warning_locale = find_field(&presentation, "/result/warning/locale")
            .expect("warning locale field should exist");

        assert_eq!(
            intent_locale.get("value").and_then(Value::as_str),
            Some("en_DE")
        );
        assert_eq!(
            warning_locale.get("value").and_then(Value::as_str),
            Some("en-DE")
        );
    }

    #[test]
    fn venue_labels_cover_mapped_and_unmapped_values() {
        assert_eq!(display_venue_label("GETTEX"), VENUE_LABEL_GETTEX);
        assert_eq!(display_venue_label("XETR"), VENUE_LABEL_XETRA);
        assert_eq!(display_venue_label("SEIX"), VENUE_LABEL_SEIX);
        assert_eq!(display_venue_label("OTCX"), "OTCX");
    }

    #[test]
    fn buy_regulatory_disclosures_include_xetra_market_data_notice() {
        let disclosures = build_buy_regulatory_disclosures("XETR", VENUE_LABEL_XETRA);

        assert_eq!(
            disclosures.market_data_notice,
            Some(BUY_XETRA_MARKET_DATA_NOTICE)
        );
        assert_eq!(
            disclosures.execution_instruction,
            "I instruct Scalable Capital to place this order for execution on Xetra valid for up to 360 days."
        );
        assert_eq!(
            disclosures.ex_ante_costs_notice,
            Some(BUY_EX_ANTE_COSTS_NOTICE)
        );
    }

    #[test]
    fn buy_regulatory_disclosures_omit_market_data_notice_for_non_xetra() {
        let disclosures = build_buy_regulatory_disclosures("SEIX", VENUE_LABEL_SEIX);

        assert_eq!(disclosures.market_data_notice, None);
        assert_eq!(
            disclosures.execution_instruction,
            "I instruct Scalable Capital to place this order for execution on European Investor Exchange (EIX) valid for up to 360 days."
        );
        assert_eq!(
            disclosures.ex_ante_costs_notice,
            Some(BUY_EX_ANTE_COSTS_NOTICE)
        );
    }

    #[test]
    fn sell_regulatory_disclosures_include_execution_instruction_only() {
        let disclosures = build_sell_regulatory_disclosures("GETTEX", VENUE_LABEL_GETTEX);

        assert_eq!(disclosures.market_data_notice, None);
        assert_eq!(
            disclosures.execution_instruction,
            "I instruct Scalable Capital to place this order for execution on Börse München (gettex) valid for up to 360 days."
        );
        assert_eq!(disclosures.ex_ante_costs_notice, None);
    }

    #[test]
    fn buy_presentation_includes_regulatory_disclosures_section_after_ex_ante_costs() {
        let presentation =
            build_phase1_presentation(&sample_result_payload(), &sample_confirmation_payload())
                .expect("presentation should build");

        let section_order = presentation
            .get("section_order")
            .and_then(Value::as_array)
            .expect("section order");
        let ex_ante_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("ex_ante_costs"))
            .expect("ex-ante section");
        let price_warnings_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("price_warnings"))
            .expect("price warnings section");
        let disclosures_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("regulatory_disclosures"))
            .expect("regulatory disclosures section");
        let suitability_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("suitability"))
            .expect("suitability section");
        let document_links_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("document_links"))
            .expect("document links section");
        let confirmation_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("confirmation"))
            .expect("confirmation section");

        assert_eq!(price_warnings_idx + 1, ex_ante_idx);
        assert_eq!(suitability_idx, ex_ante_idx + 1);
        assert_eq!(disclosures_idx, suitability_idx + 1);
        assert_eq!(document_links_idx, disclosures_idx + 1);
        assert_eq!(confirmation_idx, document_links_idx + 1);
        assert!(
            find_field(
                &presentation,
                "/result/regulatory_disclosures/execution_instruction"
            )
            .is_some()
        );
        assert!(find_field(&presentation, "/result/price_warnings/items").is_some());
        assert!(find_field(&presentation, "/result/document_links/client_documents").is_some());
    }

    #[test]
    fn sell_presentation_includes_regulatory_disclosures_section_after_ex_ante_costs() {
        let presentation = build_phase1_presentation(
            &sample_sell_result_payload(),
            &sample_confirmation_payload(),
        )
        .expect("presentation should build");

        let section_order = presentation
            .get("section_order")
            .and_then(Value::as_array)
            .expect("section order");
        let ex_ante_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("ex_ante_costs"))
            .expect("ex-ante section");
        let price_warnings_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("price_warnings"))
            .expect("price warnings section");
        let disclosures_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("regulatory_disclosures"))
            .expect("regulatory disclosures section");
        let suitability_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("suitability"))
            .expect("suitability section");
        let document_links_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("document_links"))
            .expect("document links section");
        let confirmation_idx = section_order
            .iter()
            .position(|item| item.as_str() == Some("confirmation"))
            .expect("confirmation section");

        assert_eq!(price_warnings_idx + 1, ex_ante_idx);
        assert_eq!(suitability_idx, ex_ante_idx + 1);
        assert_eq!(disclosures_idx, suitability_idx + 1);
        assert_eq!(document_links_idx, disclosures_idx + 1);
        assert_eq!(confirmation_idx, document_links_idx + 1);
        assert!(
            find_field(
                &presentation,
                "/result/regulatory_disclosures/execution_instruction"
            )
            .is_some()
        );
        assert_eq!(
            find_field(
                &presentation,
                "/result/regulatory_disclosures/execution_instruction"
            )
            .and_then(|field| field.get("value"))
            .and_then(Value::as_str),
            Some(
                "I instruct Scalable Capital to place this order for execution on Börse München (gettex) valid for up to 360 days."
            )
        );
        assert_eq!(
            find_field(&presentation, "/result/document_links/client_documents")
                .and_then(|field| field.get("value"))
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str),
            Some(CLIENT_DOCUMENTS_URL)
        );
        assert_eq!(
            find_field(&presentation, "/result/document_links/secondary_kid")
                .and_then(|field| field.get("value"))
                .and_then(Value::as_null),
            Some(())
        );
    }

    #[test]
    fn render_trade_buy_text_uses_label_and_disclosures() {
        let mut result = sample_result_payload();
        result["price_warnings"] = json!({
            "items": [{
                "code": "limit_buy_immediate_execution",
                "message": "Your LIMIT is above the current price. The order will be executed immediately.",
                "trigger_field": "limit_price",
                "reference_price_type": "ask",
                "reference_price": "50.5000"
            }]
        });
        let payload = json!({
            "result": result,
            "confirmation": sample_confirmation_payload(),
            "compliance": {
                "instruction": "Present values before phase 2."
            },
            "next_step": "confirm_with_id"
        });

        let lines = render_trade_buy_text(&payload);

        assert!(
            lines
                .iter()
                .any(|line| { line == "selected_venue: European Investor Exchange (EIX)" })
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "quote_ask_price: 50.5000 EUR")
        );
        assert!(lines.iter().any(|line| {
            line == "  limit_buy_immediate_execution: Your LIMIT is above the current price. The order will be executed immediately. (reference ask 50.5000)"
        }));
        assert!(lines.iter().any(|line| {
            line == "execution_instruction: I instruct Scalable Capital to place this order for execution on European Investor Exchange (EIX) valid for up to 360 days."
        }));
        assert!(lines.iter().any(|line| {
            *line == format!("client_documents: Client documents - {CLIENT_DOCUMENTS_URL}")
        }));
        assert!(lines.iter().any(|line| {
            line == "primary_kid: Key information document (KID) - https://example.test/primary-kid.pdf"
        }));
        assert!(
            lines.iter().any(|line| {
                *line == format!("ex_ante_costs_notice: {BUY_EX_ANTE_COSTS_NOTICE}")
            })
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "  Five years costs amount: 2.50")
        );
        assert!(lines.iter().any(|line| {
            line == "  Five years costs percentage (raw fraction, 1 = 100%): 0.00550"
        }));
        let ex_ante_idx = lines
            .iter()
            .position(|line| line == "Ex-ante costs")
            .expect("ex-ante block");
        let warnings_idx = lines
            .iter()
            .position(|line| line == "Price warnings")
            .expect("warnings block");
        let confirmation_idx = lines
            .iter()
            .position(|line| line == "confirmation_id: scb1_test")
            .expect("confirmation line");
        let execution_idx = lines
            .iter()
            .position(|line| line.starts_with("execution_instruction: I instruct Scalable Capital"))
            .expect("execution instruction");
        let suitability_idx = lines
            .iter()
            .position(|line| line == "Suitability")
            .expect("suitability block");
        let document_links_idx = lines
            .iter()
            .position(|line| line == "Document links")
            .expect("document links block");

        assert!(warnings_idx < ex_ante_idx);
        assert!(lines[ex_ante_idx - 1].is_empty());
        assert!(lines[confirmation_idx - 1].is_empty());
        assert!(ex_ante_idx < suitability_idx);
        assert!(suitability_idx < execution_idx);
        assert!(execution_idx < document_links_idx);
        assert!(document_links_idx < confirmation_idx);
        assert!(
            lines
                .iter()
                .any(|line| line == "phase_2_acknowledgement_flag: null")
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("market_data_notice:"))
        );
    }

    #[test]
    fn render_trade_cancel_text_shows_requested_order_id() {
        let lines = render_trade_cancel_text(&json!({
            "result": {
                "order_id": "order-1",
                "accepted": true
            }
        }));

        assert_eq!(lines, vec!["Cancellation requested.", "order_id: order-1"]);
    }

    #[test]
    fn render_trade_buy_text_shows_unsuitable_acknowledgement_section() {
        let mut result = sample_result_payload();
        result["suitability"] = json!({
            "source": "legacy_appropriateness_fallback",
            "type": "LEGACY_FALLBACK",
            "status": "UNSUITABLE",
            "action_when_unsuitable": "PROCEED_TO_ORDER_FLOW",
            "questionnaire_required": false,
            "questionnaire_reason": Value::Null,
            "requires_accept_unsuitable": true,
            "accept_flag": "--accept-unsuitable"
        });
        let payload = json!({
            "result": result,
            "confirmation": sample_confirmation_payload(),
            "compliance": {
                "instruction": "Present values before phase 2."
            },
            "next_step": "confirm_with_id"
        });

        let lines = render_trade_buy_text(&payload);
        let suitability_idx = lines
            .iter()
            .position(|line| line == "Suitability")
            .expect("suitability block");

        assert!(lines[suitability_idx + 1] == "source: legacy_appropriateness_fallback");
        assert!(lines[suitability_idx + 3] == "status: UNSUITABLE");
        assert!(
            lines
                .iter()
                .any(|line| line == "phase_2_acknowledgement_flag: --accept-unsuitable")
        );
    }

    #[test]
    fn render_trade_buy_text_shows_knockout_warning_section() {
        let mut result = sample_result_payload();
        result["suitability"] = json!({
            "source": "suitability_service",
            "type": "KNOCKOUT",
            "status": "SUITABLE",
            "action_when_unsuitable": "PROCEED_TO_ORDER_FLOW",
            "questionnaire_required": false,
            "questionnaire_reason": Value::Null,
            "requires_accept_unsuitable": false,
            "accept_flag": Value::Null
        });
        result["warning"] = json!({
            "kind": "knockout_risk_warning",
            "title": "Risk warning",
            "body": "On average, 7 out of 10 retail investors incur losses when trading turbo certificates. Turbo certificates are high-risk products and are not suitable for long-term investment strategies.",
            "locale": "en_DE",
            "version": Value::Null,
            "acknowledgement_text": Value::Null
        });
        let payload = json!({
            "result": result,
            "confirmation": sample_confirmation_payload(),
            "compliance": {
                "instruction": "Present values before phase 2."
            },
            "next_step": "confirm_with_id"
        });

        let lines = render_trade_buy_text(&payload);

        assert!(lines.iter().any(|line| line == "Warning"));
        assert!(
            lines
                .iter()
                .any(|line| line == "kind: knockout_risk_warning")
        );
        assert!(lines.iter().any(|line| line == "title: Risk warning"));
        assert!(lines.iter().any(|line| {
            line.starts_with("body: On average, 7 out of 10 retail investors incur losses")
        }));
    }

    #[test]
    fn questionnaire_name_for_error_uses_appropriateness_for_legacy_fallback() {
        let mut decision = crate::trade::TradeComplianceDecision::not_required();
        decision.source_kind =
            crate::trade::TradeComplianceSourceKind::LegacyAppropriatenessFallback;
        decision.suitability_type = Some(crate::trade::TradeSuitabilityType::LegacyFallback);

        assert_eq!(questionnaire_name_for_error(&decision), "appropriateness");
    }

    #[test]
    fn render_trade_sell_text_uses_label_without_buy_disclosures() {
        let payload = json!({
            "result": sample_sell_result_payload(),
            "confirmation": sample_confirmation_payload(),
            "compliance": {
                "instruction": "Present values before phase 2."
            },
            "next_step": "confirm_with_id"
        });

        let lines = render_trade_sell_text(&payload);

        assert!(
            lines
                .iter()
                .any(|line| line == "selected_venue: Börse München (gettex)")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "quote_bid_price: 50.4000 EUR")
        );
        assert!(!lines.iter().any(|line| line.starts_with("amount: ")));
        let ex_ante_idx = lines
            .iter()
            .position(|line| line == "Ex-ante costs")
            .expect("ex-ante block");
        let confirmation_idx = lines
            .iter()
            .position(|line| line == "confirmation_id: scb1_test")
            .expect("confirmation line");
        let suitability_idx = lines
            .iter()
            .position(|line| line == "Suitability")
            .expect("suitability block");
        let document_links_idx = lines
            .iter()
            .position(|line| line == "Document links")
            .expect("document links block");

        assert!(lines[ex_ante_idx - 1].is_empty());
        assert!(lines[confirmation_idx - 1].is_empty());
        assert!(ex_ante_idx < suitability_idx);
        assert!(suitability_idx < document_links_idx);
        assert!(document_links_idx < confirmation_idx);
        assert!(
            lines.iter().any(|line| {
                line == "execution_instruction: I instruct Scalable Capital to place this order for execution on Börse München (gettex) valid for up to 360 days."
            })
        );
        assert!(lines.iter().any(|line| {
            *line == format!("client_documents: Client documents - {CLIENT_DOCUMENTS_URL}")
        }));
        assert!(lines.iter().any(|line| {
            line == "primary_kid: Key information document (KID) - https://example.test/primary-kid.pdf"
        }));
        assert!(
            lines
                .iter()
                .any(|line| line == "  Five years costs amount: 2.50")
        );
        assert!(lines.iter().any(|line| {
            line == "  Five years costs percentage (raw fraction, 1 = 100%): 0.00550"
        }));
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("market_data_notice:"))
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("ex_ante_costs_notice:"))
        );
        assert!(!lines.iter().any(|line| line == "Price warnings"));
    }

    fn sample_prepared_trade_for_phase2_validation() -> PreparedTrade {
        PreparedTrade {
            env: TargetEnv::Prod,
            account_id: "account-1".to_string(),
            portfolio_id: "portfolio-1".to_string(),
            account_source: "context",
            portfolio_source: "context",
            intent: TradeIntent {
                side: TradeSide::Buy,
                isin: "DE0007100000".to_string(),
                amount: Some(500.0),
                amount_str: Some("500".to_string()),
                shares: Some(9.0),
                shares_str: Some("9".to_string()),
                order_type: ORDER_TYPE_LIMIT.to_string(),
                limit_price: Some(48.5),
                stop_price: None,
                limit_price_str: Some("48.50".to_string()),
                stop_price_str: None,
                venue_override: Some("GETTEX".to_string()),
                locale: "en_DE".to_string(),
            },
            tradability_gate: TradeTradabilityGate {
                status: "TRADABLE_WITHOUT_APPROPRIATENESS".to_string(),
                tradable: true,
                requires_appropriateness: false,
                selected_venue: "GETTEX".to_string(),
                selected_venue_status: "TRADABLE_WITHOUT_APPROPRIATENESS".to_string(),
                selected_venue_unavailability_reason: None,
                selected_venue_sellable: None,
            },
            compliance_decision: crate::trade::TradeComplianceDecision::not_required(),
            warning_json: json!({
                "kind": "legacy_appropriateness_warning",
                "title": Value::Null,
                "body": "Prompt",
                "locale": "en_DE",
                "version": "v1",
                "acknowledgement_text": "Ack"
            }),
            warning_version_for_order: Some("v1".to_string()),
            appropriateness_id_for_order: None,
            quote_mid_price_value: 50.4561,
            quote_ask_price_value: Some(50.5000),
            quote_bid_price_value: Some(50.4000),
            quote_mid_price: "50.4561".to_string(),
            quote_ask_price: Some("50.5000".to_string()),
            quote_bid_price: Some("50.4000".to_string()),
            quote_currency: "EUR".to_string(),
            quote_timestamp_utc: Some("2026-03-10T19:25:29.000Z".to_string()),
            selected_venue_label: VENUE_LABEL_GETTEX.to_string(),
            primary_kid_url: Some("https://example.test/primary-kid.pdf".to_string()),
            secondary_kid_url: Some("https://example.test/en-kid.pdf".to_string()),
            sizing_price_basis: "ask_price",
            sizing_price: "50.5000".to_string(),
            estimate_price_basis: "limit_price",
            estimate_price: "48.5000".to_string(),
            number_of_shares: 9.0,
            number_of_shares_str: "9".to_string(),
            is_whole_position_sold: false,
            estimated_order_volume_raw: 454.1049,
            estimated_order_volume: 454.1049,
            ex_ante_costs: json!({
                "id": "cost-id",
                "entryCosts": {
                    "serviceCosts": {"amount": 0.12, "percentage": 0.00026},
                    "productCosts": {"amount": 0.34, "percentage": 0.00075},
                    "total": {"amount": 0.46, "percentage": 0.00101}
                },
                "ongoingCosts": {
                    "serviceCosts": {"amount": 0.05, "percentage": 0.00011},
                    "productCosts": {"amount": 0.07, "percentage": 0.00015},
                    "total": {"amount": 0.12, "percentage": 0.00026}
                },
                "exitCosts": {
                    "serviceCosts": {"amount": 0.08, "percentage": 0.00018},
                    "productCosts": {"amount": 0.09, "percentage": 0.00019},
                    "total": {"amount": 0.17, "percentage": 0.00037}
                },
                "effectOnReturn": {
                    "initialYearCosts": {"amount": 0.46, "percentage": 0.00101},
                    "followingYearsCosts": {"amount": 0.12, "percentage": 0.00026},
                    "finalYearCosts": {"amount": 0.17, "percentage": 0.00037}
                },
                "incidentalCosts": {"amount": 0.03, "percentage": 0.00007},
                "fiveYearsCosts": {"amount": 2.50, "percentage": 0.00550}
            }),
            confirmation_fields: ConfirmationFields {
                isin: "DE0007100000".to_string(),
                amount: Some("500".to_string()),
                currency: "EUR".to_string(),
                venue: "GETTEX".to_string(),
                shares: "9".to_string(),
                entry_total: "0.46".to_string(),
                ongoing_total: "0.12".to_string(),
                exit_total: "0.17".to_string(),
                five_years_total: "2.5".to_string(),
            },
            snapshot_payload: json!({}),
            intent_checksum: "checksum".to_string(),
        }
    }

    fn sample_tradability_gate_for_calculation(side: TradeSide) -> TradeTradabilityGate {
        TradeTradabilityGate {
            status: "TRADABLE_WITHOUT_APPROPRIATENESS".to_string(),
            tradable: true,
            requires_appropriateness: false,
            selected_venue: "GETTEX".to_string(),
            selected_venue_status: "TRADABLE_WITHOUT_APPROPRIATENESS".to_string(),
            selected_venue_unavailability_reason: None,
            selected_venue_sellable: matches!(side, TradeSide::Sell).then_some(5.0),
        }
    }

    #[test]
    fn calculate_trade_quantities_sizes_buy_amount_from_ask_and_estimates_limit_from_limit_price() {
        let intent = TradeIntent {
            side: TradeSide::Buy,
            isin: "DE0007100000".to_string(),
            amount: Some(500.0),
            amount_str: Some("500".to_string()),
            shares: None,
            shares_str: None,
            order_type: ORDER_TYPE_LIMIT.to_string(),
            limit_price: Some(48.5),
            stop_price: None,
            limit_price_str: Some("48.50".to_string()),
            stop_price_str: None,
            venue_override: None,
            locale: "en_DE".to_string(),
        };
        let quote = SecurityTick {
            ask_price: Some(50.5),
            bid_price: Some(50.4),
            mid_price: 50.4561,
            currency: "EUR".to_string(),
            is_outdated: false,
            timestamp_utc: None,
        };

        let calculation = calculate_trade_quantities(
            &intent,
            &quote,
            &sample_tradability_gate_for_calculation(TradeSide::Buy),
        )
        .expect("calculation should succeed");

        assert_eq!(calculation.sizing_price_basis, "ask_price");
        assert_eq!(calculation.sizing_price_value, 50.5);
        assert_eq!(calculation.estimate_price_basis, "limit_price");
        assert_eq!(calculation.estimate_price_value, 48.5);
        assert_eq!(calculation.number_of_shares, 9.0);
        assert_eq!(calculation.number_of_shares_str, "9");
        assert_eq!(calculation.estimated_order_volume_raw, 436.5);
        assert_eq!(calculation.estimated_order_volume, 436.5);
    }

    #[test]
    fn calculate_trade_quantities_uses_bid_for_sell_stop_estimate() {
        let intent = TradeIntent {
            side: TradeSide::Sell,
            isin: "DE0007100000".to_string(),
            amount: None,
            amount_str: None,
            shares: Some(5.0),
            shares_str: Some("5".to_string()),
            order_type: ORDER_TYPE_STOP.to_string(),
            limit_price: None,
            stop_price: Some(48.0),
            limit_price_str: None,
            stop_price_str: Some("48.00".to_string()),
            venue_override: None,
            locale: "en_DE".to_string(),
        };
        let quote = SecurityTick {
            ask_price: Some(50.5),
            bid_price: Some(50.4),
            mid_price: 50.4561,
            currency: "EUR".to_string(),
            is_outdated: false,
            timestamp_utc: None,
        };

        let calculation = calculate_trade_quantities(
            &intent,
            &quote,
            &sample_tradability_gate_for_calculation(TradeSide::Sell),
        )
        .expect("calculation should succeed");

        assert_eq!(calculation.sizing_price_basis, "bid_price");
        assert_eq!(calculation.estimate_price_basis, "stop_price");
        assert_eq!(calculation.number_of_shares, 5.0);
        assert!(calculation.is_whole_position_sold);
        assert_eq!(calculation.estimated_order_volume_raw, 240.0);
        assert_eq!(calculation.estimated_order_volume, 240.0);
    }

    #[test]
    fn calculate_trade_quantities_fails_closed_when_sell_bid_is_missing() {
        let intent = TradeIntent {
            side: TradeSide::Sell,
            isin: "DE0007100000".to_string(),
            amount: None,
            amount_str: None,
            shares: Some(5.0),
            shares_str: Some("5".to_string()),
            order_type: ORDER_TYPE_MARKET.to_string(),
            limit_price: None,
            stop_price: None,
            limit_price_str: None,
            stop_price_str: None,
            venue_override: None,
            locale: "en_DE".to_string(),
        };
        let quote = SecurityTick {
            ask_price: Some(50.5),
            bid_price: None,
            mid_price: 50.4561,
            currency: "EUR".to_string(),
            is_outdated: false,
            timestamp_utc: None,
        };

        let err = calculate_trade_quantities(
            &intent,
            &quote,
            &sample_tradability_gate_for_calculation(TradeSide::Sell),
        )
        .expect_err("missing bid should fail");

        assert!(err.to_string().contains("missing bidPrice"));
    }

    fn sample_trade_confirmation_for_phase2_validation(
        prepared: &PreparedTrade,
    ) -> TradeConfirmation {
        TradeConfirmation {
            confirmation_id: "scb1_test".to_string(),
            intent_checksum: prepared.intent_checksum.clone(),
            nonce: "nonce".to_string(),
            created_at_epoch: 1,
            expires_at_epoch: 2,
            consumed_at_epoch: None,
            env: prepared.env.as_str().to_string(),
            account_id: prepared.account_id.clone(),
            portfolio_id: prepared.portfolio_id.clone(),
            side: ORDER_SIDE_BUY.to_string(),
            order_type: prepared.intent.order_type.clone(),
            locale: prepared.intent.locale.clone(),
            venue_override: prepared.intent.venue_override.clone(),
            warning_version: prepared.warning_version_for_order.clone(),
            requires_accept_unsuitable: false,
            phase1_input: ConfirmationPhase1Input {
                side: ORDER_SIDE_BUY.to_string(),
                isin: prepared.intent.isin.clone(),
                amount: prepared.intent.amount_str.clone(),
                shares: prepared.intent.shares_str.clone(),
                venue: prepared.intent.venue_override.clone(),
                order_type: prepared.intent.order_type.clone(),
                limit_price: prepared.intent.limit_price_str.clone(),
                stop_price: prepared.intent.stop_price_str.clone(),
            },
            fields: prepared.confirmation_fields.clone(),
            snapshot_payload: prepared.snapshot_payload.clone(),
            ex_ante_costs: prepared.ex_ante_costs.clone(),
        }
    }

    #[test]
    fn phase2_snapshot_check_rejects_ex_ante_drift_without_total_drift() {
        let mut prepared = sample_prepared_trade_for_phase2_validation();
        let stored = sample_trade_confirmation_for_phase2_validation(&prepared);

        prepared.ex_ante_costs["entryCosts"]["serviceCosts"]["amount"] = json!(0.11);
        prepared.ex_ante_costs["entryCosts"]["productCosts"]["amount"] = json!(0.35);

        let err = ensure_phase2_snapshot_matches(&prepared, &stored)
            .expect_err("ex-ante drift should fail");
        assert!(
            err.to_string()
                .contains("fresh ex-ante costs differ from phase 1 snapshot")
        );
    }

    #[test]
    fn phase2_snapshot_check_allows_percentage_only_ex_ante_drift() {
        let mut prepared = sample_prepared_trade_for_phase2_validation();
        let stored = sample_trade_confirmation_for_phase2_validation(&prepared);

        prepared.ex_ante_costs["entryCosts"]["serviceCosts"]["percentage"] = json!(0.00027);
        prepared.ex_ante_costs["effectOnReturn"]["finalYearCosts"]["percentage"] = json!(0.00038);

        ensure_phase2_snapshot_matches(&prepared, &stored)
            .expect("percentage-only ex-ante drift should be ignored");
    }

    #[test]
    fn phase2_snapshot_check_ignores_non_amount_ex_ante_metadata_drift() {
        let mut prepared = sample_prepared_trade_for_phase2_validation();
        let stored = sample_trade_confirmation_for_phase2_validation(&prepared);

        prepared.ex_ante_costs["id"] = json!("cost-id-updated");

        ensure_phase2_snapshot_matches(&prepared, &stored)
            .expect("non-amount ex-ante metadata drift should be ignored");
    }

    #[test]
    fn phase2_snapshot_check_canonicalizes_ex_ante_amount_formatting() {
        let prepared = sample_prepared_trade_for_phase2_validation();
        let mut stored = sample_trade_confirmation_for_phase2_validation(&prepared);

        stored.ex_ante_costs["entryCosts"]["serviceCosts"]["amount"] = json!("0.1200");
        stored.ex_ante_costs["fiveYearsCosts"]["amount"] = json!("2.5000");

        ensure_phase2_snapshot_matches(&prepared, &stored)
            .expect("equivalent ex-ante amount formatting should be ignored");
    }

    #[test]
    fn phase2_snapshot_check_allows_matching_absence_of_nullable_effect_on_return_amounts() {
        let mut prepared = sample_prepared_trade_for_phase2_validation();
        let mut stored = sample_trade_confirmation_for_phase2_validation(&prepared);

        prepared.ex_ante_costs["effectOnReturn"]["initialYearCosts"] = Value::Null;
        prepared.ex_ante_costs["effectOnReturn"]["followingYearsCosts"] = Value::Null;
        stored.ex_ante_costs["effectOnReturn"]["initialYearCosts"] = Value::Null;
        stored.ex_ante_costs["effectOnReturn"]["followingYearsCosts"] = Value::Null;

        ensure_phase2_snapshot_matches(&prepared, &stored)
            .expect("matching nullable effect-on-return absence should be allowed");
    }

    #[test]
    fn phase2_snapshot_check_rejects_required_final_year_amount_disappearance() {
        let prepared = sample_prepared_trade_for_phase2_validation();
        let mut stored = sample_trade_confirmation_for_phase2_validation(&prepared);

        stored.ex_ante_costs["effectOnReturn"]["finalYearCosts"]
            .as_object_mut()
            .expect("final year costs object")
            .remove("amount");

        let err = ensure_phase2_snapshot_matches(&prepared, &stored)
            .expect_err("missing final year amount should fail");
        assert!(
            err.to_string()
                .contains("fresh ex-ante costs differ from phase 1 snapshot")
        );
    }

    #[test]
    fn phase2_snapshot_check_rejects_suitability_acknowledgement_drift() {
        let mut prepared = sample_prepared_trade_for_phase2_validation();
        let stored = sample_trade_confirmation_for_phase2_validation(&prepared);

        prepared.compliance_decision.requires_accept_unsuitable = true;
        prepared.compliance_decision.status = crate::trade::TradeSuitabilityStatus::Unsuitable;

        let err = ensure_phase2_snapshot_matches(&prepared, &stored)
            .expect_err("suitability drift should fail");
        assert!(
            err.to_string()
                .contains("fresh suitability acknowledgement requirement differs")
        );
    }

    #[test]
    fn phase2_submission_requirements_prefer_snapshot_mismatch_over_ack_error() {
        let prepared = sample_prepared_trade_for_phase2_validation();
        let mut stored = sample_trade_confirmation_for_phase2_validation(&prepared);
        stored.requires_accept_unsuitable = true;
        let phase2 = Phase2Input {
            confirmation_id: "scb1_test".to_string(),
            accept_unsuitable: false,
            isin: "DE0007100000".to_string(),
            amount: Some("500".to_string()),
            amount_value: Some(500.0),
            shares: None,
            shares_value: None,
            venue: None,
            order_type: ORDER_TYPE_MARKET.to_string(),
            limit_price: None,
            stop_price: None,
            limit_price_value: None,
            stop_price_value: None,
        };

        let err = ensure_phase2_submission_requirements(&prepared, &phase2, &stored)
            .expect_err("suitability drift should fail before ack requirement");
        assert!(
            err.to_string()
                .contains("fresh suitability acknowledgement requirement differs")
        );
    }

    #[test]
    fn canonical_cost_from_json_returns_na_for_missing_entry_total() {
        assert_eq!(canonical_cost_from_json(None), "n/a");
        assert_eq!(canonical_cost_from_json(Some(&Value::Null)), "n/a");
    }

    #[test]
    fn build_phase2_intent_uses_phase2_values() {
        let phase2 = Phase2Input {
            confirmation_id: "scb1_test".to_string(),
            accept_unsuitable: false,
            isin: "de0007100000".to_string(),
            amount: Some("500.00".to_string()),
            amount_value: Some(500.0),
            shares: None,
            shares_value: None,
            venue: Some(" gettex ".to_string()),
            order_type: ORDER_TYPE_MARKET.to_string(),
            limit_price: None,
            stop_price: None,
            limit_price_value: None,
            stop_price_value: None,
        };
        let phase1 = ConfirmationPhase1Input {
            side: ORDER_SIDE_BUY.to_string(),
            isin: "de0007100000".to_string(),
            amount: Some("500.00".to_string()),
            shares: None,
            venue: Some(" gettex ".to_string()),
            order_type: ORDER_TYPE_MARKET.to_string(),
            limit_price: None,
            stop_price: None,
        };

        let intent =
            build_phase2_intent(TradeSide::Buy, &phase2, &phase1).expect("intent should build");

        assert_eq!(intent.isin, "DE0007100000");
        assert_eq!(intent.amount_str.as_deref(), Some("500.00"));
        assert_eq!(intent.venue_override.as_deref(), Some(" GETTEX "));
        assert_eq!(intent.locale, TRADE_WARNING_LOCALE);
    }

    #[test]
    fn build_phase2_intent_normalizes_warning_query_locale_without_mutating_intent_locale() {
        let phase2 = Phase2Input {
            confirmation_id: "scb1_test".to_string(),
            accept_unsuitable: false,
            isin: "de0007100000".to_string(),
            amount: Some("500.00".to_string()),
            amount_value: Some(500.0),
            shares: None,
            shares_value: None,
            venue: None,
            order_type: ORDER_TYPE_MARKET.to_string(),
            limit_price: None,
            stop_price: None,
            limit_price_value: None,
            stop_price_value: None,
        };
        let phase1 = ConfirmationPhase1Input {
            side: ORDER_SIDE_BUY.to_string(),
            isin: "de0007100000".to_string(),
            amount: Some("500.00".to_string()),
            shares: None,
            venue: None,
            order_type: ORDER_TYPE_MARKET.to_string(),
            limit_price: None,
            stop_price: None,
        };

        let intent =
            build_phase2_intent(TradeSide::Buy, &phase2, &phase1).expect("intent should build");
        let warning_variables =
            trade_appropriateness_warning_variables(&intent.locale).expect("warning vars");

        assert_eq!(intent.locale, TRADE_WARNING_LOCALE);
        assert_eq!(warning_variables["locale"], "en-DE");
    }

    #[test]
    fn resolve_sell_trade_shares_marks_whole_position_when_shares_match_sellable() {
        let (shares, shares_str, whole) =
            resolve_sell_trade_shares(2.5, Some("2.5"), "MUNC", Some(2.5))
                .expect("shares should resolve");

        assert_eq!(shares, 2.5);
        assert_eq!(shares_str, "2.5");
        assert!(whole);
    }

    #[test]
    fn resolve_sell_trade_shares_marks_partial_sale_when_shares_are_below_sellable() {
        let (shares, shares_str, whole) =
            resolve_sell_trade_shares(2.0, Some("2"), "MUNC", Some(2.5))
                .expect("shares should resolve");

        assert_eq!(shares, 2.0);
        assert_eq!(shares_str, "2");
        assert!(!whole);
    }

    #[test]
    fn resolve_sell_trade_shares_rejects_requested_shares_above_sellable() {
        let err = resolve_sell_trade_shares(3.0, Some("3"), "MUNC", Some(2.5))
            .expect_err("oversell should fail");
        assert!(
            err.to_string()
                .contains("shares 3 exceed sellable quantity 2.5 on venue 'MUNC'")
        );
    }

    #[test]
    fn resolve_sell_trade_shares_rejects_missing_selected_venue_sellable() {
        let err = resolve_sell_trade_shares(2.0, Some("2"), "MUNC", None)
            .expect_err("missing sellable should fail");
        assert!(
            err.to_string()
                .contains("missing sellable quantity for selected venue 'MUNC'")
        );
    }

    #[test]
    fn assert_phase2_matches_phase1_rejects_limit_price_mismatch() {
        let phase2 = Phase2Input {
            confirmation_id: "scb1_test".to_string(),
            accept_unsuitable: false,
            isin: "DE0007100000".to_string(),
            amount: Some("500".to_string()),
            amount_value: Some(500.0),
            shares: None,
            shares_value: None,
            venue: None,
            order_type: ORDER_TYPE_LIMIT.to_string(),
            limit_price: Some("123.45".to_string()),
            stop_price: None,
            limit_price_value: Some(123.45),
            stop_price_value: None,
        };
        let phase1 = ConfirmationPhase1Input {
            side: ORDER_SIDE_BUY.to_string(),
            isin: "DE0007100000".to_string(),
            amount: Some("500".to_string()),
            shares: None,
            venue: None,
            order_type: ORDER_TYPE_LIMIT.to_string(),
            limit_price: Some("120".to_string()),
            stop_price: None,
        };

        let err = assert_phase2_matches_phase1_input(TradeSide::Buy, &phase2, &phase1)
            .expect_err("limit mismatch should fail");
        assert!(err.to_string().contains("--limit-price does not match"));
    }

    #[test]
    fn assert_phase2_matches_phase1_rejects_stop_price_mismatch() {
        let phase2 = Phase2Input {
            confirmation_id: "scb1_test".to_string(),
            accept_unsuitable: false,
            isin: "DE0007100000".to_string(),
            amount: Some("500".to_string()),
            amount_value: Some(500.0),
            shares: None,
            shares_value: None,
            venue: None,
            order_type: ORDER_TYPE_STOP.to_string(),
            limit_price: None,
            stop_price: Some("88.10".to_string()),
            limit_price_value: None,
            stop_price_value: Some(88.10),
        };
        let phase1 = ConfirmationPhase1Input {
            side: ORDER_SIDE_BUY.to_string(),
            isin: "DE0007100000".to_string(),
            amount: Some("500".to_string()),
            shares: None,
            venue: None,
            order_type: ORDER_TYPE_STOP.to_string(),
            limit_price: None,
            stop_price: Some("87.50".to_string()),
        };

        let err = assert_phase2_matches_phase1_input(TradeSide::Buy, &phase2, &phase1)
            .expect_err("stop mismatch should fail");
        assert!(err.to_string().contains("--stop-price does not match"));
    }

    #[test]
    fn ensure_unsuitable_acknowledgement_requires_accept_unsuitable_when_stored() {
        let prepared = sample_prepared_trade_for_phase2_validation();
        let mut stored = sample_trade_confirmation_for_phase2_validation(&prepared);
        stored.requires_accept_unsuitable = true;
        let phase2 = Phase2Input {
            confirmation_id: "scb1_test".to_string(),
            accept_unsuitable: false,
            isin: "DE0007100000".to_string(),
            amount: Some("500".to_string()),
            amount_value: Some(500.0),
            shares: None,
            shares_value: None,
            venue: None,
            order_type: ORDER_TYPE_MARKET.to_string(),
            limit_price: None,
            stop_price: None,
            limit_price_value: None,
            stop_price_value: None,
        };

        let err = ensure_unsuitable_acknowledgement(&phase2, &stored)
            .expect_err("missing ack should fail");
        assert!(err.to_string().contains("--accept-unsuitable"));
    }

    #[test]
    fn validate_order_price_flags_accepts_limit() {
        let prices = validate_order_price_flags(
            crate::cli::BrokerTradeOrderType::Limit,
            Some("123.45"),
            None,
        )
        .expect("limit prices should validate");
        assert_eq!(prices.limit_price.as_deref(), Some("123.45"));
        assert_eq!(prices.stop_price, None);
    }

    #[test]
    fn parse_phase1_intent_buy_accepts_derivative_isin() {
        let args = crate::cli::BrokerTradeBuyArgs {
            isin: Some("DE000HSBC123".to_string()),
            amount: Some("500".to_string()),
            order_type: crate::cli::BrokerTradeOrderType::Market,
            limit_price: None,
            stop_price: None,
            venue: Some("gettex".to_string()),
            confirm: None,
            accept_unsuitable: false,
            json: true,
        };

        let intent = parse_phase1_intent_buy(&args).expect("phase 1 intent");
        let confirmation =
            parse_phase1_input_for_confirmation_buy(&args).expect("phase 1 confirmation input");

        assert_eq!(intent.isin, "DE000HSBC123");
        assert_eq!(intent.venue_override.as_deref(), Some("GETTEX"));
        assert_eq!(confirmation.isin, "DE000HSBC123");
    }

    #[test]
    fn validate_order_price_flags_rejects_market_with_prices() {
        let err = validate_order_price_flags(
            crate::cli::BrokerTradeOrderType::Market,
            Some("123.45"),
            None,
        )
        .expect_err("market with limit should fail");
        assert!(err.to_string().contains("--order-type market"));
    }

    #[test]
    fn validate_order_price_flags_rejects_both_prices() {
        let err = validate_order_price_flags(
            crate::cli::BrokerTradeOrderType::Limit,
            Some("123.45"),
            Some("120"),
        )
        .expect_err("both prices should fail");
        assert!(err.to_string().contains("cannot be used together"));
    }

    #[test]
    fn validate_order_price_flags_requires_stop_for_stop_order() {
        let err = validate_order_price_flags(crate::cli::BrokerTradeOrderType::Stop, None, None)
            .expect_err("missing stop should fail");
        assert!(err.to_string().contains("--stop-price is required"));
    }

    #[test]
    fn ensure_price_warning_quote_legs_rejects_buy_limit_without_ask() {
        let intent = TradeIntent {
            side: TradeSide::Buy,
            isin: "DE0007100000".to_string(),
            amount: Some(500.0),
            amount_str: Some("500".to_string()),
            shares: None,
            shares_str: None,
            order_type: ORDER_TYPE_LIMIT.to_string(),
            limit_price: Some(48.5),
            stop_price: None,
            limit_price_str: Some("48.50".to_string()),
            stop_price_str: None,
            venue_override: Some("SEIX".to_string()),
            locale: "en_DE".to_string(),
        };
        let quote = SecurityTick {
            ask_price: None,
            bid_price: Some(48.2),
            mid_price: 48.3,
            currency: "EUR".to_string(),
            is_outdated: false,
            timestamp_utc: Some("2026-03-10T19:25:29.000Z".to_string()),
        };

        let err = ensure_price_warning_quote_legs(&intent, &quote)
            .expect_err("missing ask price should fail closed");
        assert_eq!(
            err.to_string(),
            "PRICE_WARNING_UNAVAILABLE: missing askPrice required for buy limit warning evaluation"
        );
    }

    #[test]
    fn ensure_price_warning_quote_legs_rejects_sell_limit_without_bid() {
        let intent = TradeIntent {
            side: TradeSide::Sell,
            isin: "DE0007100000".to_string(),
            amount: None,
            amount_str: None,
            shares: Some(9.0),
            shares_str: Some("9".to_string()),
            order_type: ORDER_TYPE_LIMIT.to_string(),
            limit_price: Some(48.5),
            stop_price: None,
            limit_price_str: Some("48.50".to_string()),
            stop_price_str: None,
            venue_override: Some("SEIX".to_string()),
            locale: "en_DE".to_string(),
        };
        let quote = SecurityTick {
            ask_price: Some(48.8),
            bid_price: None,
            mid_price: 48.6,
            currency: "EUR".to_string(),
            is_outdated: false,
            timestamp_utc: Some("2026-03-10T19:25:29.000Z".to_string()),
        };

        let err = ensure_price_warning_quote_legs(&intent, &quote)
            .expect_err("missing bid price should fail closed");
        assert_eq!(
            err.to_string(),
            "PRICE_WARNING_UNAVAILABLE: missing bidPrice required for sell limit warning evaluation"
        );
    }

    #[test]
    fn phase2_snapshot_check_ignores_missing_warning_only_quote_legs() {
        let mut prepared = sample_prepared_trade_for_phase2_validation();
        let stored = sample_trade_confirmation_for_phase2_validation(&prepared);

        prepared.quote_ask_price_value = None;
        prepared.quote_ask_price = None;
        prepared.quote_bid_price_value = None;
        prepared.quote_bid_price = None;

        ensure_phase2_snapshot_matches(&prepared, &stored)
            .expect("phase 2 snapshot checks should ignore warning-only quote legs");
    }

    #[test]
    fn execute_broker_trade_cancel_happy_path_returns_acceptance_payload() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config = sample_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = crate::session::SessionManager::new(&config).expect("session");
        session_manager
            .save_active(&sample_stored_session(crate::channel::current_env()))
            .expect("save session");

        let cancel_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_authorization_header())
            .match_body(Matcher::Regex("BrokerCancelOrder".to_string()))
            .match_body(Matcher::PartialJson(json!({
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

        let payload = execute_broker_trade_cancel(
            crate::cli::BrokerTradeCancelArgs {
                order_id: " order-1 ".to_string(),
                portfolio_id: Some("portfolio-1".to_string()),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect("cancel payload");

        assert_eq!(
            payload.get("account_id").and_then(Value::as_str),
            Some("person-1")
        );
        assert_eq!(
            payload
                .pointer("/resolution/account")
                .and_then(Value::as_str),
            Some("auto_session_person_id")
        );
        assert_eq!(
            payload
                .pointer("/resolution/portfolio")
                .and_then(Value::as_str),
            Some("explicit")
        );
        assert_eq!(
            payload.pointer("/result/order_id").and_then(Value::as_str),
            Some("order-1")
        );
        assert_eq!(payload.pointer("/result/accepted"), Some(&json!(true)));

        cancel_mock.assert();
    }

    #[test]
    fn execute_broker_trade_cancel_rejects_blank_order_id_before_session_lookup() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config = sample_config();
        let mut session_manager = crate::session::SessionManager::new(&config).expect("session");

        let err = execute_broker_trade_cancel(
            crate::cli::BrokerTradeCancelArgs {
                order_id: "   ".to_string(),
                portfolio_id: None,
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect_err("blank order_id should fail before session lookup");

        assert_eq!(
            err.to_string(),
            "Trade input invalid: field 'order_id' must be a non-empty string"
        );
    }
}
