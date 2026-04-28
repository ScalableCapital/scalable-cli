use anyhow::Result;
use serde_json::{Value, json};

use crate::broker_shared::{
    ResolvedBrokerIds, checksum_for_payload, fingerprint_payload_for_transactions_input,
    load_active_session, resolve_broker_ids, validated_broker_input,
};
use crate::config::AppConfig;
use crate::graphql::execute_graphql;
use crate::helpers::{
    BROKER_ANALYTICS_QUERY, BROKER_HOLDINGS_QUERY, BROKER_LIMITS_QUERY, BROKER_OVERVIEW_QUERY,
    BROKER_PRICE_ALERTS_QUERY, BROKER_QUOTE_QUERY, BROKER_SAVINGS_PLANS_QUERY, BROKER_SEARCH_QUERY,
    BROKER_SECURITY_NEWS_QUERY, BROKER_TRANSACTION_DETAILS_QUERY, BROKER_TRANSACTIONS_QUERY,
    BROKER_WATCHLIST_QUERY, broker_analytics_variables, broker_holdings_variables,
    broker_limits_variables, broker_overview_variables, broker_price_alerts_variables,
    broker_quote_variables, broker_savings_plans_variables, broker_search_variables,
    broker_transaction_details_variables, broker_transactions_variables_from_normalized,
    broker_watchlist_variables, normalize_broker_transactions_query_input,
    project_broker_analytics_response, project_broker_holdings_response,
    project_broker_limits_response, project_broker_overview_response,
    project_broker_price_alerts_response, project_broker_quote_response,
    project_broker_savings_plans_response, project_broker_search_response,
    project_broker_security_news_response, project_broker_transaction_details_response,
    project_broker_transactions_response, project_broker_watchlist_response,
};
use crate::session::SessionManager;
use crate::{execute_with_refresh_retry, resolve_active_env};

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

pub(crate) fn execute_broker_overview(
    args: crate::cli::BrokerOverviewArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, args.include_year_to_date, None)?;
    let variables = broker_overview_variables(&input)?;
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
                BROKER_OVERVIEW_QUERY,
                &variables,
                Some("BrokerOverview"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_overview_response(&input, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_analytics(
    args: crate::cli::BrokerAnalyticsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, false, None)?;
    let variables = broker_analytics_variables(&input)?;
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
                BROKER_ANALYTICS_QUERY,
                &variables,
                Some("BrokerAnalytics"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_analytics_response(&input, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_transactions(
    args: crate::cli::BrokerTransactionsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let normalized_query = normalize_broker_transactions_query_input(
        args.page_size,
        args.cursor.as_deref(),
        &args.type_filter,
        &args.status,
        args.search_term.as_deref(),
        args.from_time.as_deref(),
        args.to_time.as_deref(),
        args.isin.as_deref(),
        args.include_reinvestment_subtypes,
    )?;
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, false, None)?;
    let variables = broker_transactions_variables_from_normalized(&input, &normalized_query);
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
                BROKER_TRANSACTIONS_QUERY,
                &variables,
                Some("BrokerTransactions"),
                dpop_options,
            )
        },
    )?;
    let mut projected = project_broker_transactions_response(&input, &response)?;

    let normalized_input = variables.get("input").cloned().unwrap_or(Value::Null);
    let fingerprint_payload = fingerprint_payload_for_transactions_input(&normalized_input);
    let input_fingerprint = checksum_for_payload(&fingerprint_payload);
    if let Some(result) = projected.as_object_mut() {
        result.insert("input".to_string(), normalized_input);
        result.insert(
            "input_fingerprint".to_string(),
            Value::String(input_fingerprint),
        );
    }

    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_transaction_details(
    args: crate::cli::BrokerTransactionDetailsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, false, None)?;
    let variables = broker_transaction_details_variables(&input, &args.transaction_id)?;
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
                BROKER_TRANSACTION_DETAILS_QUERY,
                &variables,
                Some("BrokerTransactionDetails"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_transaction_details_response(&input, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_holdings(
    args: crate::cli::BrokerHoldingsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(
        &ids,
        args.include_year_to_date,
        args.quote_source.as_deref(),
    )?;
    let variables = broker_holdings_variables(&input)?;
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
                BROKER_HOLDINGS_QUERY,
                &variables,
                Some("BrokerHoldings"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_holdings_response(&input, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_watchlist(
    args: crate::cli::BrokerWatchlistArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(
        &ids,
        args.include_year_to_date,
        args.quote_source.as_deref(),
    )?;
    let variables = broker_watchlist_variables(&input)?;
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
                BROKER_WATCHLIST_QUERY,
                &variables,
                Some("BrokerWatchlist"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_watchlist_response(&input, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_search(
    args: crate::cli::BrokerSearchArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(
        &ids,
        args.include_year_to_date,
        args.quote_source.as_deref(),
    )?;
    let query = args.query.trim();
    let variables = broker_search_variables(&input, query)?;
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
                BROKER_SEARCH_QUERY,
                &variables,
                Some("BrokerSecuritySearch"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_search_response(&input, query, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_quote(
    args: crate::cli::BrokerQuoteArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(
        &ids,
        args.include_year_to_date,
        args.quote_source.as_deref(),
    )?;
    let requested_isin = args.isin.trim().to_string();
    let variables = broker_quote_variables(&input, &requested_isin)?;
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
                BROKER_QUOTE_QUERY,
                &variables,
                Some("BrokerQuote"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_quote_response(&input, &requested_isin, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_security_news(
    args: crate::cli::BrokerSecurityNewsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let locale = args
        .locale
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("en_DE")
        .to_string();
    let isin = args.isin.trim().to_string();
    let variables = json!({
        "isin": isin,
        "locale": locale,
    });
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
                BROKER_SECURITY_NEWS_QUERY,
                &variables,
                Some("BrokerSecurityNews"),
                dpop_options,
            )
        },
    )?;
    project_broker_security_news_response(&isin, &locale, &response)
}

pub(crate) fn execute_broker_price_alerts(
    args: crate::cli::BrokerPriceAlertsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let crate::cli::BrokerPriceAlertsArgs {
        command: _,
        portfolio_id,
        active_only,
        json: _,
    } = args;

    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, false, None)?;
    let variables = broker_price_alerts_variables(&input, active_only)?;
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
                BROKER_PRICE_ALERTS_QUERY,
                &variables,
                Some("BrokerPriceAlerts"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_price_alerts_response(&input, active_only, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_limits(
    args: crate::cli::BrokerLimitsArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        args.portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, false, None)?;
    let variables = broker_limits_variables(&input)?;
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
                BROKER_LIMITS_QUERY,
                &variables,
                Some("BrokerLimits"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_limits_response(&input, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

pub(crate) fn execute_broker_savings_plans(
    args: crate::cli::BrokerSavingsPlansArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let crate::cli::BrokerSavingsPlansArgs {
        command: _,
        portfolio_id,
        json: _,
    } = args;

    let env = resolve_active_env(session_manager)?;
    let env_cfg = crate::channel::current_env_config();
    let mut session = load_active_session(session_manager, env, &env_cfg, dpop_options)?;
    let ids = resolve_broker_ids(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        portfolio_id.as_deref(),
    )?;
    let input = validated_broker_input(&ids, false, None)?;
    let variables = broker_savings_plans_variables(&input)?;
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
                BROKER_SAVINGS_PLANS_QUERY,
                &variables,
                Some("BrokerSavingsPlans"),
                dpop_options,
            )
        },
    )?;
    let projected = project_broker_savings_plans_response(&input, &response)?;
    Ok(broker_result_envelope(&ids, projected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server};

    use crate::cli::{
        BrokerOverviewArgs, BrokerQuoteArgs, BrokerSearchArgs, BrokerTransactionsArgs,
    };
    use crate::config::TargetEnv;
    use crate::session::{LoginSource, Session, StoredSession};

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
            source: LoginSource::DeviceCode,
        }
    }

    fn sample_stored_session(env: TargetEnv) -> StoredSession {
        StoredSession {
            env,
            session: sample_session(),
            dpop_jwk_thumbprint: Some(current_runtime_dpop_thumbprint()),
        }
    }

    fn sample_config() -> AppConfig {
        AppConfig {
            auth: crate::config::RuntimeAuthConfig {
                session_backend: crate::config::SessionBackendPreference::File,
                signing_key_backend: crate::config::DpopKeyBackend::File,
                pkcs11: None,
            },
        }
    }

    fn expected_authorization_header() -> &'static str {
        "DPoP test-token"
    }

    fn sample_overview_args() -> BrokerOverviewArgs {
        BrokerOverviewArgs {
            portfolio_id: None,
            include_year_to_date: true,
            json: true,
        }
    }

    fn sample_search_args() -> BrokerSearchArgs {
        BrokerSearchArgs {
            query: "tesla".to_string(),
            portfolio_id: Some("portfolio-1".to_string()),
            include_year_to_date: true,
            quote_source: Some("CONSOLIDATED".to_string()),
            json: true,
        }
    }

    fn sample_quote_args() -> BrokerQuoteArgs {
        BrokerQuoteArgs {
            portfolio_id: Some("portfolio-1".to_string()),
            isin: "US0378331005".to_string(),
            include_year_to_date: true,
            quote_source: Some("CONSOLIDATED".to_string()),
            json: true,
        }
    }

    fn sample_transactions_args() -> BrokerTransactionsArgs {
        BrokerTransactionsArgs {
            portfolio_id: Some("portfolio-1".to_string()),
            page_size: 5,
            cursor: Some("cursor-123".to_string()),
            type_filter: vec!["BUY".to_string()],
            status: vec!["FILLED".to_string()],
            search_term: Some("monthly".to_string()),
            from_time: Some("2026-01-01T00:00:00Z".to_string()),
            to_time: Some("2026-02-01T00:00:00Z".to_string()),
            isin: Some("DE0007100000".to_string()),
            include_reinvestment_subtypes: true,
            json: true,
        }
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

    #[test]
    fn execute_broker_overview_happy_path_wraps_projected_result_in_resolution_envelope() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config = sample_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = SessionManager::new(&config).expect("session manager");
        session_manager
            .save_active(&sample_stored_session(crate::channel::current_env()))
            .expect("save session");
        let expected_auth_header = expected_authorization_header();

        let resolve_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_auth_header)
            .match_body(Matcher::Regex("ResolveBrokerIds".to_string()))
            .match_body(Matcher::PartialJson(json!({
                "variables": {
                    "id": "person-1"
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "account": {
                            "id": "account-1",
                            "brokerPortfolios": [{ "id": "portfolio-1" }]
                        }
                    }
                }"#,
            )
            .create();

        let overview_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_auth_header)
            .match_body(Matcher::Regex("BrokerOverview".to_string()))
            .match_body(Matcher::PartialJson(json!({
                "variables": {
                    "accountId": "account-1",
                    "portfolioId": "portfolio-1",
                    "includeYearToDate": true
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "account": {
                            "brokerPortfolio": {
                                "valuation": {
                                    "valuation": 1234.56,
                                    "securitiesValuation": 1200.00,
                                    "cryptoValuation": 34.56,
                                    "timestampUtc": "2026-03-10T19:25:29Z",
                                    "lastInventoryUpdateTimestampUtc": "2026-03-10T18:00:00Z",
                                    "timeWeightedReturnByTimeframe": [{"timeframe":"ONE_DAY","value":0.01}]
                                }
                            }
                        }
                    }
                }"#,
            )
            .create();

        let payload =
            execute_broker_overview(sample_overview_args(), &config, &mut session_manager)
                .expect("overview payload");

        assert_eq!(
            payload.get("account_id").and_then(Value::as_str),
            Some("account-1")
        );
        assert_eq!(
            payload.get("portfolio_id").and_then(Value::as_str),
            Some("portfolio-1")
        );
        assert_eq!(
            payload
                .pointer("/resolution/account")
                .and_then(Value::as_str),
            Some("auto_resolve")
        );
        assert_eq!(
            payload
                .pointer("/resolution/portfolio")
                .and_then(Value::as_str),
            Some("auto_resolve")
        );
        assert_eq!(
            payload.pointer("/result/valuation/total"),
            Some(&json!(1234.56))
        );
        assert_eq!(
            payload
                .pointer("/result/timestamps/valuation_timestamp_utc")
                .and_then(Value::as_str),
            Some("2026-03-10T19:25:29Z")
        );
        resolve_mock.assert();
        overview_mock.assert();
    }

    #[test]
    fn execute_broker_transactions_happy_path_includes_envelope_input_and_fingerprint() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config = sample_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = SessionManager::new(&config).expect("session manager");
        session_manager
            .save_active(&sample_stored_session(crate::channel::current_env()))
            .expect("save session");
        let expected_auth_header = expected_authorization_header();

        let args = sample_transactions_args();
        let transactions_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_auth_header)
            .match_body(Matcher::Regex("BrokerTransactions".to_string()))
            .match_body(Matcher::PartialJson(json!({
                "variables": {
                    "accountId": "person-1",
                    "portfolioId": "portfolio-1",
                    "input": {
                        "pageSize": 5,
                        "cursor": "cursor-123",
                        "type": ["BUY"],
                        "status": ["FILLED"],
                        "searchTerm": "monthly",
                        "fromTime": 1767225600u64,
                        "toTime": 1769904000u64,
                        "isin": "DE0007100000",
                        "includeReinvestmentSubtypes": true
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "account": {
                            "brokerPortfolio": {
                                "moreTransactions": {
                                    "cursor": "next-cursor",
                                    "total": 1,
                                    "transactions": [
                                        {
                                            "__typename": "BrokerSecurityTransactionSummary",
                                            "id": "txn-1",
                                            "currency": "EUR",
                                            "type": "TRADE",
                                            "status": "FILLED",
                                            "isCancellation": false,
                                            "lastEventDateTime": "2026-01-10T12:30:00Z",
                                            "description": "Monthly investment",
                                            "custodian": "BROKER",
                                            "documents": [],
                                            "isin": "DE0007100000",
                                            "securityTransactionType": "BUY",
                                            "quantity": 2.5,
                                            "amount": 250.0,
                                            "side": "BUY",
                                            "limitPrice": null,
                                            "stopPrice": null
                                        }
                                    ]
                                }
                            }
                        }
                    }
                }"#,
            )
            .create();

        let payload =
            execute_broker_transactions(args, &config, &mut session_manager).expect("payload");

        assert_eq!(
            payload.get("account_id").and_then(Value::as_str),
            Some("person-1")
        );
        assert_eq!(
            payload
                .pointer("/resolution/portfolio")
                .and_then(Value::as_str),
            Some("explicit")
        );
        assert_eq!(
            payload.pointer("/result/cursor").and_then(Value::as_str),
            Some("next-cursor")
        );
        assert_eq!(payload.pointer("/result/count"), Some(&json!(1)));
        assert_eq!(
            payload
                .pointer("/result/items/0/isin")
                .and_then(Value::as_str),
            Some("DE0007100000")
        );

        let expected_input = json!({
            "cursor": "cursor-123",
            "pageSize": 5,
            "type": ["BUY"],
            "status": ["FILLED"],
            "searchTerm": "monthly",
            "fromTime": 1767225600u64,
            "toTime": 1769904000u64,
            "isin": "DE0007100000",
            "includeReinvestmentSubtypes": true
        });
        assert_eq!(payload.pointer("/result/input"), Some(&expected_input));

        let expected_fingerprint =
            checksum_for_payload(&fingerprint_payload_for_transactions_input(&expected_input));
        assert_eq!(
            payload
                .pointer("/result/input_fingerprint")
                .and_then(Value::as_str),
            Some(expected_fingerprint.as_str())
        );
        transactions_mock.assert();
    }

    #[test]
    fn execute_broker_search_happy_path_wraps_sorted_results_in_resolution_envelope() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config = sample_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = SessionManager::new(&config).expect("session manager");
        session_manager
            .save_active(&sample_stored_session(crate::channel::current_env()))
            .expect("save session");
        let expected_auth_header = expected_authorization_header();

        let search_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_auth_header)
            .match_body(Matcher::Regex("BrokerSecuritySearch".to_string()))
            .match_body(Matcher::PartialJson(json!({
                "variables": {
                    "accountId": "person-1",
                    "portfolioId": "portfolio-1",
                    "includeYearToDate": true,
                    "quoteSource": "CONSOLIDATED",
                    "searchTerm": "tesla"
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": {
                        "account": {
                            "brokerPortfolio": {
                                "simpleSecuritySearch": {
                                    "items": [
                                        {
                                            "isin": "US88160R1014",
                                            "name": "Tesla",
                                            "type": "EQ",
                                            "quoteTick": {
                                                "midPrice": 200.11,
                                                "currency": "USD",
                                                "timestampUtc": "2026-03-11T08:00:00Z",
                                                "isOutdated": false
                                            }
                                        },
                                        {
                                            "isin": "DE0007100000",
                                            "name": "Mercedes-Benz Group",
                                            "type": "EQ",
                                            "quoteTick": {
                                                "midPrice": 58.3,
                                                "currency": "EUR",
                                                "timestampUtc": "2026-03-11T08:01:00Z",
                                                "isOutdated": false
                                            }
                                        }
                                    ]
                                }
                            }
                        }
                    }
                }"#,
            )
            .create();

        let payload = execute_broker_search(sample_search_args(), &config, &mut session_manager)
            .expect("search payload");

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
            payload.pointer("/result/query").and_then(Value::as_str),
            Some("tesla")
        );
        assert_eq!(payload.pointer("/result/count"), Some(&json!(2)));
        assert_eq!(
            payload
                .pointer("/result/items/0/isin")
                .and_then(Value::as_str),
            Some("DE0007100000")
        );
        assert_eq!(
            payload
                .pointer("/result/items/1/isin")
                .and_then(Value::as_str),
            Some("US88160R1014")
        );
        search_mock.assert();
    }

    #[test]
    fn execute_broker_quote_happy_path_wraps_quote_in_resolution_envelope() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut server = Server::new();
        let _channel_guard = TestChannelGuard::for_server(&server);
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        let config = sample_config();
        ensure_runtime_dpop_key(&config);
        let mut session_manager = SessionManager::new(&config).expect("session manager");
        session_manager
            .save_active(&sample_stored_session(crate::channel::current_env()))
            .expect("save session");
        let expected_auth_header = expected_authorization_header();

        let quote_mock = server
            .mock("POST", "/")
            .match_header("authorization", expected_auth_header)
            .match_body(Matcher::Regex("BrokerQuote".to_string()))
            .match_body(Matcher::PartialJson(json!({
                "variables": {
                    "accountId": "person-1",
                    "portfolioId": "portfolio-1",
                    "isin": "US0378331005",
                    "includeYearToDate": true,
                    "quoteSource": "CONSOLIDATED"
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
                                    "id": "security-1",
                                    "isin": "US0378331005",
                                    "name": "Apple",
                                    "type": "EQ",
                                    "quoteTick": {
                                        "id": "tick-1",
                                        "isin": "US0378331005",
                                        "midPrice": 201.1,
                                        "currency": "USD",
                                        "bidPrice": 201.0,
                                        "askPrice": 201.2,
                                        "isOutdated": false,
                                        "timestampUtc": {
                                            "time": "2026-03-11T08:00:00Z"
                                        },
                                        "performanceDate": {
                                            "date": "2026-03-11"
                                        },
                                        "performancesByTimeframe": [
                                            {
                                                "timeframe": "ONE_DAY",
                                                "performance": 0.01,
                                                "simpleAbsoluteReturn": 2.0
                                            }
                                        ]
                                    }
                                }
                            }
                        }
                    }
                }"#,
            )
            .create();

        let payload = execute_broker_quote(sample_quote_args(), &config, &mut session_manager)
            .expect("quote payload");

        assert_eq!(
            payload.get("account_id").and_then(Value::as_str),
            Some("person-1")
        );
        assert_eq!(
            payload
                .pointer("/resolution/portfolio")
                .and_then(Value::as_str),
            Some("explicit")
        );
        assert_eq!(
            payload.pointer("/result/isin").and_then(Value::as_str),
            Some("US0378331005")
        );
        assert_eq!(
            payload
                .pointer("/result/security_id")
                .and_then(Value::as_str),
            Some("security-1")
        );
        assert_eq!(
            payload.pointer("/result/quote_mid_price"),
            Some(&json!(201.1))
        );
        assert_eq!(
            payload
                .pointer("/result/quote_tick_id")
                .and_then(Value::as_str),
            Some("tick-1")
        );
        assert_eq!(
            payload
                .pointer("/result/quote_timestamp_utc")
                .and_then(Value::as_str),
            Some("2026-03-11T08:00:00Z")
        );
        assert_eq!(
            payload.pointer("/result/quote_performances/0/timeframe"),
            Some(&json!("ONE_DAY"))
        );
        quote_mock.assert();
    }
}
