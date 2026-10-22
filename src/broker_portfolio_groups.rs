use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashSet;

use crate::active_session::load_active_session;
use crate::broker_portfolio_groups_projections::project_broker_portfolio_groups_response;
use crate::broker_portfolio_groups_queries::{
    BROKER_ASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION, BROKER_CREATE_PORTFOLIO_GROUP_MUTATION,
    BROKER_DELETE_PORTFOLIO_GROUP_MUTATION, BROKER_PORTFOLIO_GROUPS_QUERY,
    BROKER_UNASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION, BROKER_UPDATE_PORTFOLIO_GROUP_MUTATION,
    broker_assign_portfolio_group_items_variables, broker_create_portfolio_group_variables,
    broker_delete_portfolio_group_variables, broker_portfolio_groups_variables,
    broker_unassign_portfolio_group_items_variables, broker_update_portfolio_group_variables,
};
use crate::broker_queries::normalize_broker_isin;
use crate::broker_shared::{
    BROKER_PORTFOLIO_GROUP_NOT_FOUND_ERROR_PREFIX, ResolvedBrokerIds, broker_result_envelope,
    resolve_broker_ids, validated_broker_input,
};
use crate::config::{AppConfig, EnvConfig, TargetEnv};
use crate::dpop::DpopRuntimeOptions;
use crate::graphql::{GraphqlAccessContext, enforce_graphql_access_policy, execute_graphql};
use crate::resolve_active_env;
use crate::session::{Session, SessionManager};
use crate::session_refresh::execute_with_refresh_retry;

struct PortfolioGroupsContext {
    env: TargetEnv,
    env_cfg: EnvConfig,
    dpop_options: DpopRuntimeOptions,
    session: Session,
    access_context: GraphqlAccessContext,
    ids: ResolvedBrokerIds,
}

#[derive(Debug, Clone, Copy)]
enum PortfolioGroupMembershipAction {
    Assign,
    Unassign,
}

impl PortfolioGroupMembershipAction {
    fn result_label(self) -> &'static str {
        match self {
            Self::Assign => "assign",
            Self::Unassign => "unassign",
        }
    }
}

struct PortfolioGroupMembershipOperation {
    action: PortfolioGroupMembershipAction,
    document: &'static str,
    operation_name: &'static str,
    variables_builder: fn(&str, &str, &[String]) -> Value,
}

const ASSIGN_PORTFOLIO_GROUP_ITEMS: PortfolioGroupMembershipOperation =
    PortfolioGroupMembershipOperation {
        action: PortfolioGroupMembershipAction::Assign,
        document: BROKER_ASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION,
        operation_name: "BrokerAssignPortfolioGroupItems",
        variables_builder: broker_assign_portfolio_group_items_variables,
    };

const UNASSIGN_PORTFOLIO_GROUP_ITEMS: PortfolioGroupMembershipOperation =
    PortfolioGroupMembershipOperation {
        action: PortfolioGroupMembershipAction::Unassign,
        document: BROKER_UNASSIGN_PORTFOLIO_GROUP_ITEMS_MUTATION,
        operation_name: "BrokerUnassignPortfolioGroupItems",
        variables_builder: broker_unassign_portfolio_group_items_variables,
    };

pub(crate) fn execute_broker_portfolio_groups(
    args: crate::cli::BrokerPortfolioGroupsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let group_id_filter = normalize_group_id_filter(args.group_id.as_deref())?;
    let mut context =
        load_portfolio_groups_context(args.portfolio_id.as_deref(), config, session_manager)?;
    let state = fetch_portfolio_groups_state(&mut context, session_manager)?;
    let result = match group_id_filter {
        Some(group_id) => project_broker_portfolio_groups_response(&state, Some(&group_id))?,
        None => project_broker_portfolio_groups_response(&state, None)?,
    };
    Ok(broker_result_envelope(&context.ids, result))
}

pub(crate) fn execute_broker_portfolio_groups_create(
    args: crate::cli::BrokerPortfolioGroupsCreateArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let name = normalize_required_input(&args.name, "name")?;
    let description = args.description.as_deref();
    let mut context =
        load_portfolio_groups_context(args.portfolio_id.as_deref(), config, session_manager)?;
    let variables =
        broker_create_portfolio_group_variables(&context.ids.portfolio_id, &name, description);
    let response = execute_portfolio_groups_operation(
        &mut context,
        session_manager,
        BROKER_CREATE_PORTFOLIO_GROUP_MUTATION,
        &variables,
        "BrokerCreatePortfolioGroup",
    )?;
    let group_id = response
        .get("createPortfolioGroup")
        .and_then(|value| value.get("portfolioGroup"))
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing createPortfolioGroup.portfolioGroup.id")
        })?;
    let state = project_broker_portfolio_groups_response(
        &fetch_portfolio_groups_state(&mut context, session_manager)?,
        None,
    )?;
    Ok(broker_result_envelope(
        &context.ids,
        json!({
            "action": "create",
            "group_id": group_id,
            "name": name,
            "description": description,
            "portfolio_groups_state": state,
        }),
    ))
}

pub(crate) fn execute_broker_portfolio_groups_update(
    args: crate::cli::BrokerPortfolioGroupsUpdateArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let group_id = normalize_required_input(&args.group_id, "group_id")?;
    let mut context =
        load_portfolio_groups_context(args.portfolio_id.as_deref(), config, session_manager)?;
    enforce_graphql_access_policy(
        BROKER_UPDATE_PORTFOLIO_GROUP_MUTATION,
        Some("BrokerUpdatePortfolioGroup"),
        context.access_context,
    )?;
    let current_state = fetch_portfolio_groups_state(&mut context, session_manager)?;
    let current = current_portfolio_group_details(&current_state, &group_id)?;
    let (name, description) = merged_portfolio_group_details(
        args.name.as_deref(),
        args.description.as_deref(),
        args.clear_description,
        current,
    )?;
    let variables = broker_update_portfolio_group_variables(
        &context.ids.portfolio_id,
        &group_id,
        &name,
        description.as_deref(),
    );
    execute_portfolio_groups_operation(
        &mut context,
        session_manager,
        BROKER_UPDATE_PORTFOLIO_GROUP_MUTATION,
        &variables,
        "BrokerUpdatePortfolioGroup",
    )?;
    let state = project_broker_portfolio_groups_response(
        &fetch_portfolio_groups_state(&mut context, session_manager)?,
        None,
    )?;
    Ok(broker_result_envelope(
        &context.ids,
        json!({
            "action": "update",
            "group_id": group_id,
            "name": name,
            "description": description,
            "portfolio_groups_state": state,
        }),
    ))
}

pub(crate) fn execute_broker_portfolio_groups_delete(
    args: crate::cli::BrokerPortfolioGroupsDeleteArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let group_id = normalize_required_input(&args.group_id, "group_id")?;
    let mut context =
        load_portfolio_groups_context(args.portfolio_id.as_deref(), config, session_manager)?;
    let variables = broker_delete_portfolio_group_variables(&context.ids.portfolio_id, &group_id);
    execute_portfolio_groups_operation(
        &mut context,
        session_manager,
        BROKER_DELETE_PORTFOLIO_GROUP_MUTATION,
        &variables,
        "BrokerDeletePortfolioGroup",
    )?;
    let state = project_broker_portfolio_groups_response(
        &fetch_portfolio_groups_state(&mut context, session_manager)?,
        None,
    )?;
    Ok(broker_result_envelope(
        &context.ids,
        json!({
            "action": "delete",
            "group_id": group_id,
            "portfolio_groups_state": state,
        }),
    ))
}

pub(crate) fn execute_broker_portfolio_groups_assign(
    args: crate::cli::BrokerPortfolioGroupsAssignArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let group_id = normalize_required_input(&args.group_id, "group_id")?;
    let isins = normalize_portfolio_group_isins(&args.isin)?;
    execute_broker_portfolio_groups_membership(
        args.portfolio_id.as_deref(),
        group_id,
        isins,
        &ASSIGN_PORTFOLIO_GROUP_ITEMS,
        config,
        session_manager,
    )
}

pub(crate) fn execute_broker_portfolio_groups_unassign(
    args: crate::cli::BrokerPortfolioGroupsUnassignArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let group_id = normalize_required_input(&args.group_id, "group_id")?;
    let isins = normalize_portfolio_group_isins(&args.isin)?;
    execute_broker_portfolio_groups_membership(
        args.portfolio_id.as_deref(),
        group_id,
        isins,
        &UNASSIGN_PORTFOLIO_GROUP_ITEMS,
        config,
        session_manager,
    )
}

fn execute_broker_portfolio_groups_membership(
    portfolio_id: Option<&str>,
    group_id: String,
    isins: Vec<String>,
    operation: &PortfolioGroupMembershipOperation,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let mut context = load_portfolio_groups_context(portfolio_id, config, session_manager)?;
    enforce_graphql_access_policy(
        operation.document,
        Some(operation.operation_name),
        context.access_context,
    )?;
    let current_state = fetch_portfolio_groups_state(&mut context, session_manager)?;
    validate_portfolio_group_membership(&current_state, &group_id, &isins, operation)?;
    let variables = (operation.variables_builder)(&context.ids.portfolio_id, &group_id, &isins);
    execute_portfolio_groups_operation(
        &mut context,
        session_manager,
        operation.document,
        &variables,
        operation.operation_name,
    )?;
    let state = project_broker_portfolio_groups_response(
        &fetch_portfolio_groups_state(&mut context, session_manager)?,
        None,
    )?;
    Ok(broker_result_envelope(
        &context.ids,
        json!({
            "action": operation.action.result_label(),
            "group_id": group_id,
            "isins": isins,
            "portfolio_groups_state": state,
        }),
    ))
}

fn load_portfolio_groups_context(
    portfolio_id: Option<&str>,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<PortfolioGroupsContext> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let loaded = load_active_session(session_manager, env, &env_cfg, &dpop_options)?;
    let mut session = loaded.session;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        &dpop_options,
        portfolio_id,
    )?;
    Ok(PortfolioGroupsContext {
        env,
        env_cfg,
        dpop_options,
        session,
        access_context: loaded.access_context,
        ids,
    })
}

fn fetch_portfolio_groups_state(
    context: &mut PortfolioGroupsContext,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let input = validated_broker_input(&context.ids, false)?;
    let variables = broker_portfolio_groups_variables(&input)?;
    execute_portfolio_groups_operation(
        context,
        session_manager,
        BROKER_PORTFOLIO_GROUPS_QUERY,
        &variables,
        "BrokerPortfolioGroups",
    )
}

fn execute_portfolio_groups_operation(
    context: &mut PortfolioGroupsContext,
    session_manager: &mut SessionManager,
    document: &str,
    variables: &Value,
    operation_name: &str,
) -> Result<Value> {
    execute_with_refresh_retry(
        session_manager,
        context.env,
        &context.env_cfg,
        &mut context.session,
        &context.dpop_options,
        |token| {
            execute_graphql(
                &context.env_cfg.graphql_url,
                token,
                document,
                variables,
                Some(operation_name),
                context.access_context,
                &context.dpop_options,
            )
        },
    )
}

#[derive(Debug)]
struct CurrentPortfolioGroupDetails {
    name: String,
    description: Option<String>,
}

fn merged_portfolio_group_details(
    requested_name: Option<&str>,
    requested_description: Option<&str>,
    clear_description: bool,
    current: CurrentPortfolioGroupDetails,
) -> Result<(String, Option<String>)> {
    let name = match requested_name {
        Some(value) => normalize_required_input(value, "name")?,
        None => current.name,
    };
    let description = if clear_description {
        None
    } else {
        requested_description
            .map(str::to_string)
            .or(current.description)
    };
    Ok((name, description))
}

fn current_portfolio_group_details(
    response: &Value,
    group_id: &str,
) -> Result<CurrentPortfolioGroupDetails> {
    let groups = response
        .get("account")
        .and_then(|value| value.get("brokerPortfolio"))
        .and_then(|value| value.get("inventory"))
        .and_then(|value| value.get("portfolioGroups"))
        .and_then(|value| value.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing inventory.portfolioGroups.items")
        })?;
    let group = groups
        .iter()
        .find(|group| group.get("id").and_then(Value::as_str) == Some(group_id))
        .ok_or_else(|| {
            anyhow!(
                "{BROKER_PORTFOLIO_GROUP_NOT_FOUND_ERROR_PREFIX} portfolio group '{group_id}' was not found in the active portfolio"
            )
        })?;
    let details = group
        .get("details")
        .ok_or_else(|| anyhow!("Broker response invalid: missing portfolio group details"))?;
    let name = details
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Broker response invalid: missing portfolio group details.name"))?
        .to_string();
    let description = match details.get("description") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.to_string()),
        Some(_) => {
            bail!(
                "Broker response invalid: portfolio group details.description must be a string or null"
            )
        }
    };
    Ok(CurrentPortfolioGroupDetails { name, description })
}

fn validate_portfolio_group_membership(
    response: &Value,
    group_id: &str,
    isins: &[String],
    operation: &PortfolioGroupMembershipOperation,
) -> Result<()> {
    let inventory = response
        .get("account")
        .and_then(|value| value.get("brokerPortfolio"))
        .and_then(|value| value.get("inventory"))
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing account.brokerPortfolio.inventory")
        })?;
    let groups = inventory
        .get("portfolioGroups")
        .and_then(|value| value.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing inventory.portfolioGroups.items")
        })?;
    let group = groups
        .iter()
        .find(|group| group.get("id").and_then(Value::as_str) == Some(group_id))
        .ok_or_else(|| {
            anyhow!(
                "{BROKER_PORTFOLIO_GROUP_NOT_FOUND_ERROR_PREFIX} portfolio group '{group_id}' was not found in the active portfolio"
            )
        })?;
    let assigned_isins = portfolio_group_item_isins(group)?;
    let mut known_isins = HashSet::new();
    for group in groups {
        known_isins.extend(portfolio_group_item_isins(group)?);
    }
    let ungrouped_items = inventory
        .get("ungroupedInventoryItems")
        .and_then(|value| value.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!("Broker response invalid: missing inventory.ungroupedInventoryItems.items")
        })?;
    known_isins.extend(portfolio_group_item_isins_from_items(ungrouped_items)?);

    let invalid_isins = isins
        .iter()
        .filter(|isin| match operation.action {
            PortfolioGroupMembershipAction::Assign => assigned_isins.contains(isin.as_str()),
            PortfolioGroupMembershipAction::Unassign => {
                known_isins.contains(isin.as_str()) && !assigned_isins.contains(isin.as_str())
            }
        })
        .map(String::as_str)
        .collect::<Vec<_>>();

    if invalid_isins.is_empty() {
        return Ok(());
    }

    let message = match operation.action {
        PortfolioGroupMembershipAction::Assign => "already assigned to",
        PortfolioGroupMembershipAction::Unassign => "not assigned to",
    };
    bail!(
        "Broker input invalid: ISINs {} portfolio group '{}': [{}]",
        message,
        group_id,
        invalid_isins.join(", ")
    )
}

fn portfolio_group_item_isins(group: &Value) -> Result<HashSet<&str>> {
    let items = group
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Broker response invalid: missing portfolio group items"))?;
    portfolio_group_item_isins_from_items(items)
}

fn portfolio_group_item_isins_from_items(items: &[Value]) -> Result<HashSet<&str>> {
    items
        .iter()
        .map(|item| {
            item.get("isin")
                .and_then(Value::as_str)
                .filter(|isin| !isin.is_empty())
                .ok_or_else(|| {
                    anyhow!("Broker response invalid: missing portfolio group item ISIN")
                })
        })
        .collect()
}

fn normalize_group_id_filter(raw: Option<&str>) -> Result<Option<String>> {
    raw.map(|value| normalize_required_input(value, "group_id"))
        .transpose()
}

fn normalize_required_input(raw: &str, field: &str) -> Result<String> {
    let value = raw.trim();
    if value.is_empty() {
        bail!("Broker input invalid: field '{field}' must be a non-empty string");
    }
    Ok(value.to_string())
}

fn normalize_portfolio_group_isins(raw_isins: &[String]) -> Result<Vec<String>> {
    let mut unique_isins = HashSet::with_capacity(raw_isins.len());
    let mut normalized = Vec::with_capacity(raw_isins.len());
    for raw_isin in raw_isins {
        let isin = normalize_broker_isin(raw_isin, "isin")?;
        if unique_isins.insert(isin.clone()) {
            normalized.push(isin);
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use mockito::{Matcher, Server};
    use serde_json::json;
    use std::ffi::OsString;

    use super::{
        ASSIGN_PORTFOLIO_GROUP_ITEMS, CurrentPortfolioGroupDetails, PortfolioGroupMembershipAction,
        UNASSIGN_PORTFOLIO_GROUP_ITEMS, current_portfolio_group_details,
        execute_broker_portfolio_groups_assign, execute_broker_portfolio_groups_create,
        execute_broker_portfolio_groups_delete, execute_broker_portfolio_groups_unassign,
        execute_broker_portfolio_groups_update, merged_portfolio_group_details,
        normalize_group_id_filter, normalize_portfolio_group_isins,
        validate_portfolio_group_membership,
    };
    use crate::cli::{
        BrokerPortfolioGroupsAssignArgs, BrokerPortfolioGroupsCreateArgs,
        BrokerPortfolioGroupsDeleteArgs, BrokerPortfolioGroupsUnassignArgs,
        BrokerPortfolioGroupsUpdateArgs,
    };
    use crate::config::{
        AppConfig, AuthConfig, DpopKeyBackend, EnvConfig, RuntimeAuthConfig,
        SessionBackendPreference,
    };
    use crate::machine::classify_error;
    use crate::session::{LoginSource, Session, SessionManager, StoredSession};

    struct EnvGuard {
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set_config_dir(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("SC_CONFIG_DIR");
            unsafe {
                std::env::set_var("SC_CONFIG_DIR", path);
            }
            Self { previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var("SC_CONFIG_DIR", value),
                    None => std::env::remove_var("SC_CONFIG_DIR"),
                }
            }
        }
    }

    fn sample_config() -> AppConfig {
        AppConfig {
            auth: RuntimeAuthConfig {
                session_backend: SessionBackendPreference::File,
                signing_key_backend: DpopKeyBackend::File,
                pkcs11: None,
            },
            trade_controls: None,
        }
    }

    fn session_manager_with_active_session(config: &AppConfig) -> SessionManager {
        crate::dpop::DpopKeyMaterial::load_or_create_for_options(
            &crate::channel::current_dpop_runtime_options(config),
        )
        .expect("create DPoP key");
        let thumbprint = crate::dpop::DpopKeyMaterial::load_existing_for_options(
            &crate::channel::current_dpop_runtime_options(config),
        )
        .expect("load DPoP key")
        .jwk_thumbprint()
        .expect("DPoP thumbprint");
        let mut session_manager = SessionManager::new(config).expect("session manager");
        session_manager
            .save_active(&StoredSession {
                env: crate::channel::current_env(),
                session: Session {
                    access_token: "test-token".to_string(),
                    refresh_token: None,
                    id_token: None,
                    expires_at: Some(9_999_999_999),
                    person_id: "person-1".to_string(),
                    source: LoginSource::DeviceCode,
                },
                dpop_jwk_thumbprint: Some(thumbprint),
                mode: None,
                refresh_recovery_pending: false,
            })
            .expect("save session");
        session_manager
    }

    fn grouped_response() -> serde_json::Value {
        json!({
            "account": {"brokerPortfolio": {"inventory": {
                "portfolioGroups": {
                    "offerAllowsAdditionalPortfolioGroup": true,
                    "maxPortfolioGroupsPerPortfolioReached": false,
                    "items": [{
                        "id": "group-1",
                        "details": {"id": "group-1", "name": "AI portfolio", "description": "tracked"},
                        "numberOfPendingOrders": 0,
                        "savingsPlansAmount": "0",
                        "performance": null,
                        "items": [{
                            "id": "item-apple",
                            "isin": "US0378331005",
                            "name": "Apple Inc.",
                            "type": "STOCK"
                        }]
                    }, {
                        "id": "group-2",
                        "details": {"id": "group-2", "name": "Former owner", "description": null},
                        "numberOfPendingOrders": 0,
                        "savingsPlansAmount": "0",
                        "performance": null,
                        "items": [{
                            "id": "item-world-etf",
                            "isin": "IE00B4ND3602",
                            "name": "MSCI World ETF",
                            "type": "ETF"
                        }]
                    }]
                },
                "ungroupedInventoryItems": {"items": []}
            }}}
        })
    }

    fn install_test_channel(server: &Server) -> crate::channel::TestEnvConfigOverrideGuard {
        crate::channel::TestEnvConfigOverrideGuard::set(EnvConfig {
            graphql_url: server.url(),
            auth: AuthConfig {
                issuer: "https://issuer.test".to_string(),
                audience: "audience".to_string(),
                client_id: "client-id".to_string(),
            },
        })
    }

    fn response_with_group(
        name: serde_json::Value,
        description: serde_json::Value,
    ) -> serde_json::Value {
        json!({
            "account": {"brokerPortfolio": {"inventory": {"portfolioGroups": {"items": [{
                "id": "group-1",
                "details": {"name": name, "description": description}
            }]}}}}
        })
    }

    #[test]
    fn normalize_group_id_filter_rejects_blank_value() {
        let err = normalize_group_id_filter(Some("   ")).expect_err("blank group id should fail");

        assert_eq!(classify_error(&err).code, "broker_input_invalid");
        assert!(err.to_string().contains("field 'group_id'"));
    }

    #[test]
    fn normalize_portfolio_group_isins_canonicalizes_validates_and_deduplicates() {
        let normalized = normalize_portfolio_group_isins(&[
            " us0378331005 ".to_string(),
            "IE00B4ND3602".to_string(),
            "US0378331005".to_string(),
        ])
        .expect("ISINs should normalize");

        assert_eq!(normalized, ["US0378331005", "IE00B4ND3602"]);
        let err = normalize_portfolio_group_isins(&["US0378331006".to_string()])
            .expect_err("invalid checksum should fail");
        assert_eq!(classify_error(&err).code, "broker_input_invalid");
    }

    #[test]
    fn membership_action_labels_remain_stable() {
        assert_eq!(
            PortfolioGroupMembershipAction::Assign.result_label(),
            "assign"
        );
        assert_eq!(
            PortfolioGroupMembershipAction::Unassign.result_label(),
            "unassign"
        );
    }

    #[test]
    fn membership_validation_accepts_real_transitions_and_rejects_no_ops() {
        let state = grouped_response();

        validate_portfolio_group_membership(
            &state,
            "group-1",
            &["IE00B4ND3602".to_string()],
            &ASSIGN_PORTFOLIO_GROUP_ITEMS,
        )
        .expect("assigning an item outside the target group should be valid");
        validate_portfolio_group_membership(
            &state,
            "group-1",
            &["US0378331005".to_string()],
            &UNASSIGN_PORTFOLIO_GROUP_ITEMS,
        )
        .expect("unassigning an item in the target group should be valid");
        validate_portfolio_group_membership(
            &state,
            "group-1",
            &["DE0007100000".to_string()],
            &UNASSIGN_PORTFOLIO_GROUP_ITEMS,
        )
        .expect("an unknown item should remain a backend validation concern");

        let assign_err = validate_portfolio_group_membership(
            &state,
            "group-1",
            &["US0378331005".to_string()],
            &ASSIGN_PORTFOLIO_GROUP_ITEMS,
        )
        .expect_err("assigning an item already in the target group should fail");
        assert_eq!(classify_error(&assign_err).code, "broker_input_invalid");
        assert!(assign_err.to_string().contains("US0378331005"));

        let unassign_err = validate_portfolio_group_membership(
            &state,
            "group-1",
            &["IE00B4ND3602".to_string()],
            &UNASSIGN_PORTFOLIO_GROUP_ITEMS,
        )
        .expect_err("unassigning an item outside the target group should fail");
        assert_eq!(classify_error(&unassign_err).code, "broker_input_invalid");
        assert!(unassign_err.to_string().contains("IE00B4ND3602"));
    }

    #[test]
    fn membership_validation_preserves_not_found_and_rejects_malformed_state() {
        let missing_group = validate_portfolio_group_membership(
            &grouped_response(),
            "missing-group",
            &["US0378331005".to_string()],
            &UNASSIGN_PORTFOLIO_GROUP_ITEMS,
        )
        .expect_err("missing group should fail");
        assert_eq!(
            classify_error(&missing_group).code,
            "portfolio_group_not_found"
        );

        let malformed_state = json!({
            "account": {"brokerPortfolio": {"inventory": {"portfolioGroups": {
                "items": [{"id": "group-1"}]
            }}}}
        });
        let malformed = validate_portfolio_group_membership(
            &malformed_state,
            "group-1",
            &["US0378331005".to_string()],
            &UNASSIGN_PORTFOLIO_GROUP_ITEMS,
        )
        .expect_err("missing group items should fail");
        assert_eq!(classify_error(&malformed).code, "broker_response_invalid");
    }

    #[test]
    fn update_merge_extracts_required_name_and_nullable_description() {
        let details = current_portfolio_group_details(
            &response_with_group(json!("AI portfolio"), json!(null)),
            "group-1",
        )
        .expect("details should parse");

        assert_eq!(details.name, "AI portfolio");
        assert_eq!(details.description, None);
    }

    #[test]
    fn update_merge_rejects_missing_or_invalid_current_name() {
        let err = current_portfolio_group_details(
            &response_with_group(json!(null), json!("tracked")),
            "group-1",
        )
        .expect_err("name is required by the backend");

        assert_eq!(classify_error(&err).code, "broker_response_invalid");
    }

    #[test]
    fn update_merge_reuses_stable_not_found_contract() {
        let err = current_portfolio_group_details(
            &response_with_group(json!("AI portfolio"), json!(null)),
            "missing-group",
        )
        .expect_err("missing group should fail");

        assert_eq!(classify_error(&err).code, "portfolio_group_not_found");
    }

    #[test]
    fn update_merge_preserves_unspecified_values_and_supports_clear_description() {
        let current = || CurrentPortfolioGroupDetails {
            name: "AI portfolio".to_string(),
            description: Some("tracked".to_string()),
        };

        assert_eq!(
            merged_portfolio_group_details(Some("Renamed"), None, false, current())
                .expect("name-only merge"),
            ("Renamed".to_string(), Some("tracked".to_string()))
        );
        assert_eq!(
            merged_portfolio_group_details(None, Some("updated"), false, current())
                .expect("description-only merge"),
            ("AI portfolio".to_string(), Some("updated".to_string()))
        );
        assert_eq!(
            merged_portfolio_group_details(None, None, true, current())
                .expect("clear-description merge"),
            ("AI portfolio".to_string(), None)
        );
    }

    #[test]
    fn redundant_membership_requests_do_not_send_mutations() {
        let _lock = crate::lock_test_env();
        let mut server = Server::new();
        let tmp = tempfile::tempdir().expect("config dir");
        let _config_dir = EnvGuard::set_config_dir(tmp.path());
        let _channel_guard = install_test_channel(&server);
        let config = sample_config();
        let mut session_manager = session_manager_with_active_session(&config);
        let preflight = server
            .mock("POST", "/")
            .match_body(Matcher::Regex("BrokerPortfolioGroups".to_string()))
            .expect(3)
            .with_status(200)
            .with_body(json!({"data": grouped_response()}).to_string())
            .create();
        let mutation = server
            .mock("POST", "/")
            .match_body(Matcher::Regex(
                "BrokerAssignPortfolioGroupItems".to_string(),
            ))
            .expect(0)
            .create();
        let unassign_mutation = server
            .mock("POST", "/")
            .match_body(Matcher::Regex(
                "BrokerUnassignPortfolioGroupItems".to_string(),
            ))
            .match_body(Matcher::PartialJson(json!({"variables": {"input": {
                "portfolioGroupItems": {"itemsToRemove": ["IE00B4ND3602"]}
            }}})))
            .expect(0)
            .create();
        let unknown_unassign = server
            .mock("POST", "/")
            .match_body(Matcher::Regex(
                "BrokerUnassignPortfolioGroupItems".to_string(),
            ))
            .match_body(Matcher::PartialJson(json!({"variables": {"input": {
                "portfolioGroupItems": {"itemsToRemove": ["DE0007100000"]}
            }}})))
            .expect(1)
            .with_status(200)
            .with_body(
                json!({"errors": [{
                    "message": "Invalid isins to add / remove from portfolio.",
                    "extensions": {"validationErrors": {"errorCode": "PortfolioGroupValidation"}}
                }]})
                .to_string(),
            )
            .create();

        let err = execute_broker_portfolio_groups_assign(
            BrokerPortfolioGroupsAssignArgs {
                group_id: "group-1".to_string(),
                isin: vec!["US0378331005".to_string(), "IE00B4ND3602".to_string()],
                portfolio_id: Some("portfolio-1".to_string()),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect_err("assigning an already assigned item should fail");

        assert_eq!(classify_error(&err).code, "broker_input_invalid");
        assert!(err.to_string().contains("US0378331005"));

        let unassign_err = execute_broker_portfolio_groups_unassign(
            BrokerPortfolioGroupsUnassignArgs {
                group_id: "group-1".to_string(),
                isin: vec!["IE00B4ND3602".to_string()],
                portfolio_id: Some("portfolio-1".to_string()),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect_err("unassigning an item outside the target group should fail");

        assert_eq!(classify_error(&unassign_err).code, "broker_input_invalid");
        assert!(unassign_err.to_string().contains("IE00B4ND3602"));

        let unknown_err = execute_broker_portfolio_groups_unassign(
            BrokerPortfolioGroupsUnassignArgs {
                group_id: "group-1".to_string(),
                isin: vec!["DE0007100000".to_string()],
                portfolio_id: Some("portfolio-1".to_string()),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect_err("an unknown item should be rejected by the backend");

        assert_eq!(classify_error(&unknown_err).code, "broker_input_invalid");
        assert!(unknown_err.to_string().contains("Invalid isins"));
        preflight.assert();
        mutation.assert();
        unassign_mutation.assert();
        unknown_unassign.assert();
    }

    #[test]
    fn mutations_execution_runs_and_returns_projected_readback() {
        let _lock = crate::lock_test_env();
        let mut server = Server::new();
        let tmp = tempfile::tempdir().expect("config dir");
        let _config_dir = EnvGuard::set_config_dir(tmp.path());
        let _channel_guard = install_test_channel(&server);
        let config = sample_config();
        let mut session_manager = session_manager_with_active_session(&config);

        let state = json!({"data": grouped_response()}).to_string();
        let create = server
            .mock("POST", "/")
            .match_body(Matcher::Regex("BrokerCreatePortfolioGroup".to_string()))
            .match_body(Matcher::PartialJson(json!({"variables": {
                "portfolioId": "portfolio-1",
                "input": {"name": "New group", "description": "new"}
            }})))
            .with_status(200)
            .with_body(
                json!({"data": {"createPortfolioGroup": {"portfolioGroup": {"id": "group-2"}}}})
                    .to_string(),
            )
            .create();
        let readback = server
            .mock("POST", "/")
            .match_body(Matcher::Regex("BrokerPortfolioGroups".to_string()))
            .expect(8)
            .with_status(200)
            .with_body(state.clone())
            .create();

        let payload = execute_broker_portfolio_groups_create(
            BrokerPortfolioGroupsCreateArgs {
                name: "New group".to_string(),
                description: Some("new".to_string()),
                portfolio_id: Some("portfolio-1".to_string()),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect("create lifecycle payload");

        assert_eq!(payload.pointer("/result/action"), Some(&json!("create")));
        assert_eq!(payload.pointer("/result/group_id"), Some(&json!("group-2")));
        assert_eq!(
            payload.pointer("/result/portfolio_groups_state/portfolio_groups/0/group_id"),
            Some(&json!("group-1"))
        );

        let update = server
            .mock("POST", "/")
            .match_body(Matcher::Regex("BrokerUpdatePortfolioGroup".to_string()))
            .match_body(Matcher::PartialJson(json!({"variables": {
                "portfolioId": "portfolio-1",
                "input": {
                    "id": "group-1",
                    "portfolioGroupDetails": {"name": "Renamed", "description": "tracked"}
                }
            }})))
            .with_status(200)
            .with_body(json!({"data": {"modifyPortfolioGroup": {"id": "portfolio-1"}}}).to_string())
            .create();
        let update_payload = execute_broker_portfolio_groups_update(
            BrokerPortfolioGroupsUpdateArgs {
                group_id: "group-1".to_string(),
                name: Some("Renamed".to_string()),
                description: None,
                clear_description: false,
                portfolio_id: Some("portfolio-1".to_string()),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect("update lifecycle payload");
        assert_eq!(
            update_payload.pointer("/result/action"),
            Some(&json!("update"))
        );

        let delete = server
            .mock("POST", "/")
            .match_body(Matcher::Regex("BrokerDeletePortfolioGroup".to_string()))
            .match_body(Matcher::PartialJson(json!({"variables": {
                "portfolioId": "portfolio-1",
                "input": {"id": "group-1"}
            }})))
            .with_status(200)
            .with_body(json!({"data": {"deletePortfolioGroup": {"id": "portfolio-1"}}}).to_string())
            .create();
        let delete_payload = execute_broker_portfolio_groups_delete(
            BrokerPortfolioGroupsDeleteArgs {
                group_id: "group-1".to_string(),
                portfolio_id: Some("portfolio-1".to_string()),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect("delete lifecycle payload");
        assert_eq!(
            delete_payload.pointer("/result/action"),
            Some(&json!("delete"))
        );

        let assign = server
            .mock("POST", "/")
            .match_body(Matcher::Regex(
                "BrokerAssignPortfolioGroupItems".to_string(),
            ))
            .match_body(Matcher::PartialJson(json!({"variables": {
                "portfolioId": "portfolio-1",
                "input": {"id": "group-1", "portfolioGroupItems": {
                    "itemsToAdd": ["IE00B4ND3602"],
                    "itemsToRemove": []
                }}
            }})))
            .with_status(200)
            .with_body(json!({"data": {"modifyPortfolioGroup": {"id": "portfolio-1"}}}).to_string())
            .create();
        let assign_payload = execute_broker_portfolio_groups_assign(
            BrokerPortfolioGroupsAssignArgs {
                group_id: "group-1".to_string(),
                isin: vec!["IE00B4ND3602".to_string()],
                portfolio_id: Some("portfolio-1".to_string()),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect("assign membership payload");
        assert_eq!(
            assign_payload.pointer("/result/action"),
            Some(&json!("assign"))
        );
        assert_eq!(
            assign_payload.pointer("/result/isins"),
            Some(&json!(["IE00B4ND3602"]))
        );

        let unassign = server
            .mock("POST", "/")
            .match_body(Matcher::Regex(
                "BrokerUnassignPortfolioGroupItems".to_string(),
            ))
            .match_body(Matcher::PartialJson(json!({"variables": {
                "portfolioId": "portfolio-1",
                "input": {"id": "group-1", "portfolioGroupItems": {
                    "itemsToAdd": [],
                    "itemsToRemove": ["US0378331005"]
                }}
            }})))
            .with_status(200)
            .with_body(json!({"data": {"modifyPortfolioGroup": {"id": "portfolio-1"}}}).to_string())
            .create();
        let unassign_payload = execute_broker_portfolio_groups_unassign(
            BrokerPortfolioGroupsUnassignArgs {
                group_id: "group-1".to_string(),
                isin: vec!["US0378331005".to_string()],
                portfolio_id: Some("portfolio-1".to_string()),
                json: true,
            },
            &config,
            &mut session_manager,
        )
        .expect("unassign membership payload");
        assert_eq!(
            unassign_payload.pointer("/result/action"),
            Some(&json!("unassign"))
        );

        create.assert();
        update.assert();
        delete.assert();
        assign.assert();
        unassign.assert();
        readback.assert();
    }
}
