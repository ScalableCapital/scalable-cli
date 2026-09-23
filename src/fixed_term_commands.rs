use anyhow::Result;
use serde_json::{Value, json};

use crate::active_session::load_active_session;
use crate::cli::FixedTermCommand;
use crate::config::AppConfig;
use crate::fixed_term_projections::{project_details, project_list};
use crate::fixed_term_queries::{DETAILS_QUERY, LIST_QUERY};
use crate::graphql::execute_graphql;
use crate::session::SessionManager;
use crate::session_refresh::execute_with_refresh_retry;

pub(crate) fn execute(
    command: FixedTermCommand,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let env = crate::resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, &dpop_options)?;
    let mut session = loaded.session;
    let mut variables = json!({"accountId": session.person_id});
    let (query, operation_name) = match &command {
        FixedTermCommand::List(_) => (LIST_QUERY, "ListFixedTermAccounts"),
        FixedTermCommand::Details(args) => {
            variables["fixedTermAccountId"] = json!(args.fixed_term_account_id);
            (DETAILS_QUERY, "GetFixedTermAccount")
        }
    };
    let response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        &dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                query,
                &variables,
                Some(operation_name),
                loaded.access_context,
                &dpop_options,
            )
        },
    )?;
    match command {
        FixedTermCommand::List(_) => project_list(&response),
        FixedTermCommand::Details(args) => project_details(&response, &args.fixed_term_account_id),
    }
}

pub(crate) fn render(payload: &Value) -> Vec<String> {
    let accounts = match payload.get("accounts").and_then(Value::as_array) {
        Some(accounts) if accounts.is_empty() => {
            return vec!["No supported Fixed-Term accounts found.".into()];
        }
        Some(accounts) => accounts.as_slice(),
        None => std::slice::from_ref(payload),
    };
    let mut lines = Vec::new();
    for account in accounts {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        if let Some(fields) = account.as_object() {
            for (name, value) in fields {
                let text = match value {
                    Value::Null => "<unavailable>".to_string(),
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                lines.push(format!("{name}: {text}"));
            }
        }
    }
    lines
}
