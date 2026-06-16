use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::broker_context::load_context as load_broker_context;
use crate::config::{EnvConfig, TargetEnv};
use crate::dpop::DpopRuntimeOptions;
use crate::graphql::{GraphqlAccessContext, execute_graphql};
use crate::helpers::BrokerInput;
use crate::session::{Session, SessionManager};
use crate::transport_security::validate_env_transport_security;
use crate::{execute_with_refresh_retry, refresh_loaded_session_if_needed};

pub(crate) const RESOLVE_BROKER_IDS_QUERY: &str = r#"
query ResolveBrokerIds($id: ID!) {
  account(id: $id) {
    id
    brokerPortfolios {
      id
    }
  }
}
"#;

pub(crate) struct ResolvedBrokerIds {
    pub(crate) account_id: String,
    pub(crate) portfolio_id: String,
    pub(crate) account_source: &'static str,
    pub(crate) portfolio_source: &'static str,
}

pub(crate) struct LoadedActiveSession {
    pub(crate) session: Session,
    pub(crate) access_context: GraphqlAccessContext,
}

pub(crate) fn fingerprint_payload_for_transactions_input(normalized_input: &Value) -> Value {
    let mut payload = normalized_input.clone();
    if let Some(obj) = payload.as_object_mut() {
        obj.remove("cursor");
    }
    payload
}

pub(crate) fn checksum_for_payload(payload: &Value) -> String {
    let canonical = canonicalize_json(payload);
    sha256_hex(canonical.to_string().as_bytes())
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = serde_json::Map::with_capacity(map.len());
            for key in keys {
                if let Some(child) = map.get(&key) {
                    sorted.insert(key, canonicalize_json(child));
                }
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        _ => value.clone(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn load_active_session(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    dpop_options: &DpopRuntimeOptions,
) -> Result<LoadedActiveSession> {
    validate_env_transport_security(env_cfg)?;
    let stored = session_manager.load_required_active()?;
    if stored.env != env {
        bail!(
            "Stored session belongs to {}, not {env}. Run 'sc login' to replace it.",
            stored.env
        );
    }
    let access_context = GraphqlAccessContext::with_session_mode(stored.mode);
    let session =
        refresh_loaded_session_if_needed(session_manager, env, env_cfg, stored, dpop_options)?;
    Ok(LoadedActiveSession {
        session,
        access_context,
    })
}

pub(crate) fn validated_broker_input(
    ids: &ResolvedBrokerIds,
    include_year_to_date: bool,
    quote_source: Option<&str>,
) -> Result<BrokerInput> {
    BrokerInput::new(
        &ids.account_id,
        &ids.portfolio_id,
        include_year_to_date,
        quote_source,
    )
}

pub(crate) fn resolve_broker_ids(
    session_manager: &mut SessionManager,
    env: TargetEnv,
    env_cfg: &EnvConfig,
    session: &mut Session,
    dpop_options: &DpopRuntimeOptions,
    explicit_portfolio_id: Option<&str>,
) -> Result<ResolvedBrokerIds> {
    let context = load_broker_context().ok().flatten();

    let mut account_source = "auto_session_person_id";
    let mut account_id = session.person_id.clone();
    if let Some(ctx) = &context
        && let Some(ctx_account) = Some(ctx.account_id.trim()).filter(|v| !v.is_empty())
    {
        account_source = "selected_context";
        account_id = ctx_account.to_string();
    }

    let mut portfolio_source = "";
    let mut portfolio_id = None::<String>;
    if let Some(explicit) = explicit_portfolio_id
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        portfolio_source = "explicit";
        portfolio_id = Some(explicit.to_string());
    } else if let Some(ctx) = &context
        && let Some(ctx_portfolio) = ctx.portfolio_id.as_deref().filter(|v| !v.is_empty())
    {
        portfolio_source = "selected_context";
        portfolio_id = Some(ctx_portfolio.to_string());
    }

    if let Some(portfolio_id) = portfolio_id {
        return Ok(ResolvedBrokerIds {
            account_id,
            portfolio_id,
            account_source,
            portfolio_source,
        });
    }

    let resolve_response = execute_with_refresh_retry(
        session_manager,
        env,
        env_cfg,
        session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                RESOLVE_BROKER_IDS_QUERY,
                &json!({ "id": account_id }),
                Some("ResolveBrokerIds"),
                GraphqlAccessContext::default(),
                dpop_options,
            )
        },
    )?;
    if let Some(resolved_account) = resolve_response
        .get("account")
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        account_source = "auto_resolve";
        account_id = resolved_account.to_string();
    }

    if portfolio_id.is_none() {
        let candidates = resolve_response
            .get("account")
            .and_then(|v| v.get("brokerPortfolios"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        portfolio_id = resolve_portfolio_id_from_candidates(candidates)?;
        if let Some(resolved) = portfolio_id {
            portfolio_source = "auto_resolve";
            portfolio_id = Some(resolved);
        }
    }

    let portfolio_id = portfolio_id.with_context(|| {
        "Unable to resolve broker portfolio id. Provide --portfolio-id or set one via `sc broker context select --portfolio-id ...`"
    })?;

    Ok(ResolvedBrokerIds {
        account_id,
        portfolio_id,
        account_source,
        portfolio_source,
    })
}

fn resolve_portfolio_id_from_candidates(mut candidates: Vec<String>) -> Result<Option<String>> {
    candidates.retain(|candidate| !candidate.trim().is_empty());
    candidates.sort();
    candidates.dedup();

    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.into_iter().next()),
        _ => bail!(
            "Unable to resolve broker portfolio id: multiple portfolios found [{}]. Provide --portfolio-id or set one via `sc broker context select --portfolio-id ...`",
            candidates.join(", ")
        ),
    }
}
