use anyhow::Result;
use serde_json::{Value, json};

use crate::active_session::load_active_session;
use crate::config::AppConfig;
use crate::graphql::execute_graphql;
use crate::overnight_projections::{
    project_overnight_discovery_response, project_overnight_summary_response,
};
use crate::overnight_queries::{
    DISCOVER_OVERNIGHT_ACCOUNTS_QUERY, OVERNIGHT_SUMMARY_QUERY, OvernightSummaryInput,
    overnight_discovery_variables, overnight_summary_variables,
};
use crate::overnight_shared::resolve_overnight_selection;
use crate::resolve_active_env;
use crate::session::SessionManager;
use crate::session_refresh::execute_with_refresh_retry;

pub(crate) fn execute_overnight_summary(
    args: crate::cli::OvernightArgs,
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

    let discovery_variables = overnight_discovery_variables(&session.person_id)?;
    let discovery_response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                DISCOVER_OVERNIGHT_ACCOUNTS_QUERY,
                &discovery_variables,
                Some("DiscoverOvernightAccounts"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let discovered_accounts = project_overnight_discovery_response(&discovery_response)?;
    let selection =
        resolve_overnight_selection(&discovered_accounts, args.savings_account_id.as_deref())?;

    let input = OvernightSummaryInput::new(&session.person_id, &selection.savings_account_id)?;
    let summary_variables = overnight_summary_variables(&input)?;
    let summary_response = execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| {
            execute_graphql(
                &env_cfg.graphql_url,
                token,
                OVERNIGHT_SUMMARY_QUERY,
                &summary_variables,
                Some("OvernightSummary"),
                access_context,
                dpop_options,
            )
        },
    )?;
    let result = project_overnight_summary_response(&input, &summary_response)?;

    Ok(json!({
        "savings_account_id": input.savings_account_id(),
        "selection": {
            "account": selection.selection_source,
        },
        "account": {
            "display_name": selection.display_name,
            "owner_kind": selection.owner_kind.as_str(),
            "is_active": selection.is_active,
        },
        "result": result,
    }))
}
