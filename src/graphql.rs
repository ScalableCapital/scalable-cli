use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::header::HeaderMap;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dpop::{DPOP_SESSION_KEY_RELOGIN_MESSAGE, DpopKeyMaterial, DpopRuntimeOptions};
use crate::session::SessionMode;
use crate::transport_security::{
    RUNTIME_HTTP_TIMEOUT, build_blocking_client_https_only_with_timeout, validate_https_url,
};

const GRAPHQL_HTTP_TIMEOUT: Duration = RUNTIME_HTTP_TIMEOUT;

const WHOAMI_QUERY: &str = r#"
query WhoAmI($id: ID!) {
  personOverview(id: $id) {
    id
    externalId
    locale
    personalDetails {
      firstName
      lastName
    }
  }
}
"#;

#[derive(Debug, Serialize)]
struct GraphqlRequest<'a, T: Serialize> {
    query: &'a str,
    variables: &'a T,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_name: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse {
    data: Option<Value>,
    errors: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GraphqlAccessContext {
    pub session_mode: Option<SessionMode>,
}

impl GraphqlAccessContext {
    pub const fn with_session_mode(session_mode: Option<SessionMode>) -> Self {
        Self { session_mode }
    }
}

struct GraphqlDpopContext {
    key_material: DpopKeyMaterial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Login2faState {
    pub enabled: Option<bool>,
    pub has_approved_session: Option<bool>,
}

impl GraphqlDpopContext {
    fn from_runtime_options(options: &DpopRuntimeOptions) -> Result<Self> {
        let key_material = DpopKeyMaterial::load_existing_for_options(options)
            .context(DPOP_SESSION_KEY_RELOGIN_MESSAGE)?;
        Ok(Self { key_material })
    }

    #[cfg(test)]
    fn with_key_material_for_tests(key_material: DpopKeyMaterial) -> Self {
        Self { key_material }
    }
}

pub(crate) const LOCAL_READ_ONLY_ERROR_PREFIX: &str = "LOCAL_READ_ONLY:";
pub(crate) const BROKER_TRANSACTION_NOT_FOUND_ERROR_PREFIX: &str = "Broker transaction not found:";

pub fn execute_graphql<T: Serialize>(
    endpoint: &str,
    token: &str,
    query: &str,
    variables: &T,
    operation_name: Option<&str>,
    access_context: GraphqlAccessContext,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Value> {
    execute_graphql_with_headers(
        endpoint,
        token,
        query,
        variables,
        operation_name,
        access_context,
        &[],
        dpop_options,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn execute_graphql_with_headers<T: Serialize>(
    endpoint: &str,
    token: &str,
    query: &str,
    variables: &T,
    operation_name: Option<&str>,
    access_context: GraphqlAccessContext,
    headers: &[(&str, &str)],
    dpop_options: &DpopRuntimeOptions,
) -> Result<Value> {
    let dpop = GraphqlDpopContext::from_runtime_options(dpop_options)?;
    execute_graphql_with_headers_with_context(
        endpoint,
        token,
        query,
        variables,
        operation_name,
        access_context,
        headers,
        &dpop,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_graphql_with_headers_with_context<T: Serialize>(
    endpoint: &str,
    token: &str,
    query: &str,
    variables: &T,
    operation_name: Option<&str>,
    access_context: GraphqlAccessContext,
    headers: &[(&str, &str)],
    dpop: &GraphqlDpopContext,
) -> Result<Value> {
    enforce_graphql_access_policy(query, operation_name, access_context)?;
    let validated_endpoint = validate_https_url(endpoint, "graphql_url")
        .with_context(|| format!("Invalid GraphQL endpoint URL: {endpoint}"))?
        .to_string();
    let client = build_blocking_client_https_only_with_timeout(GRAPHQL_HTTP_TIMEOUT)?;

    let body = GraphqlRequest {
        query,
        variables,
        operation_name,
    };

    let mut nonce = None::<String>;
    for attempt in 0..2 {
        let mut request = client
            .post(&validated_endpoint)
            .header(CONTENT_TYPE, "application/json");
        let proof = dpop
            .key_material
            .proof_for_request("POST", &validated_endpoint, nonce.as_deref(), Some(token))
            .context("Failed generating DPoP proof for GraphQL request")?;
        request = request
            .header(AUTHORIZATION, format!("DPoP {token}"))
            .header("DPoP", proof);
        request = request.json(&body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }

        let response = request.send().context("Failed to call GraphQL endpoint")?;

        if !response.status().is_success() {
            let status = response.status();
            let retry_nonce = dpop_nonce_from_headers(response.headers());
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                bail!(
                    "{}",
                    graphql_rate_limited_message(operation_name, response.headers())
                );
            }
            let text = response.text().unwrap_or_default();
            if attempt == 0 && should_retry_with_dpop_nonce(status, retry_nonce.as_deref(), &text) {
                nonce = retry_nonce;
                continue;
            }

            let mut message = format!("GraphQL HTTP error {}", status.as_u16());
            if let Some(name) = operation_name {
                message.push_str(&format!(" during {name}"));
            }
            if let Some(code) = graphql_http_error_code(&text) {
                message.push_str(&format!(" ({code})"));
            }
            if let Some(hint) = dpop_troubleshooting_hint(status, &text) {
                message.push_str(". ");
                message.push_str(hint);
            }
            bail!("{message}");
        }

        let parsed = response
            .json::<GraphqlResponse>()
            .context("Failed to parse GraphQL JSON response")?;

        if let Some(errors) = parsed.errors {
            bail!(
                "{}",
                graphql_application_error_message(operation_name, &errors)
            );
        }

        return parsed.data.context("GraphQL response missing data");
    }

    unreachable!("GraphQL retry loop should always return");
}

fn enforce_graphql_access_policy(
    query: &str,
    operation_name: Option<&str>,
    access_context: GraphqlAccessContext,
) -> Result<()> {
    if access_context.session_mode != Some(SessionMode::LocalReadOnly) {
        return Ok(());
    }

    if !is_graphql_mutation(query) {
        return Ok(());
    }

    let operation_name = operation_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "{LOCAL_READ_ONLY_ERROR_PREFIX} local read-only mode blocks write operations without an explicit allowlisted operation name"
        ))?;

    if is_local_read_only_allowed_mutation(operation_name) {
        return Ok(());
    }

    bail!(
        "{LOCAL_READ_ONLY_ERROR_PREFIX} local read-only mode blocks write operation '{operation_name}'. Re-login without --local-read-only to perform write operations."
    );
}

fn is_graphql_mutation(query: &str) -> bool {
    query.trim_start().starts_with("mutation")
}

fn is_local_read_only_allowed_mutation(operation_name: &str) -> bool {
    matches!(
        operation_name,
        "Start2faOnLogin" | "Validate2faOnLogin" | "revokeAuthAccessToken"
    )
}

fn graphql_rate_limited_message(operation_name: Option<&str>, headers: &HeaderMap) -> String {
    let mut message = String::from("RATE_LIMITED: backend rate limit exceeded");
    if let Some(name) = operation_name {
        message.push_str(&format!(" during {name}"));
    }

    if let Some(seconds) = retry_after_delta_seconds(headers) {
        message.push_str(&format!("; retry after {seconds}s"));
    } else {
        message.push_str("; retry later");
    }

    message
}

fn retry_after_delta_seconds(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
}

fn graphql_http_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body).ok().and_then(|value| {
        value
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

fn graphql_application_error_message(operation_name: Option<&str>, errors: &Value) -> String {
    if let Some(message) = broker_graphql_application_error_message(operation_name, errors) {
        return message;
    }

    let mut message = String::from("GraphQL returned errors");
    if let Some(name) = operation_name {
        message.push_str(&format!(" for {name}"));
    }
    if let Some(code) = graphql_error_code(errors) {
        message.push_str(&format!(" (code: {code})"));
    }
    message
}

fn broker_graphql_application_error_message(
    operation_name: Option<&str>,
    errors: &Value,
) -> Option<String> {
    match operation_name {
        Some("BrokerQuote")
            if graphql_error_code(errors).as_deref() == Some("BAD_USER_INPUT")
                && graphql_error_message_text(errors).as_deref()
                    == Some("Invalid ISIN provided") =>
        {
            Some("Broker input invalid: field 'isin' must be a valid ISIN".to_string())
        }
        Some("BrokerTransactionDetails")
            if graphql_validation_error_code(errors).as_deref() == Some("TransactionNotFound") =>
        {
            Some("Broker transaction not found: field 'transaction_id' was not found".to_string())
        }
        _ => None,
    }
}

fn graphql_error_code(errors: &Value) -> Option<String> {
    errors
        .as_array()
        .and_then(|items| items.first())
        .and_then(|first| first.get("extensions"))
        .and_then(|ext| ext.get("code"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn graphql_error_message_text(errors: &Value) -> Option<String> {
    errors
        .as_array()
        .and_then(|items| items.first())
        .and_then(|first| first.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn graphql_validation_error_code(errors: &Value) -> Option<String> {
    let first = errors.as_array().and_then(|items| items.first())?;
    extract_validation_error_code(first.get("validationErrors")).or_else(|| {
        first
            .get("extensions")
            .and_then(|extensions| extensions.get("validationErrors"))
            .and_then(|value| extract_validation_error_code(Some(value)))
    })
}

fn extract_validation_error_code(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Object(map) => map
            .get("errorCode")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Value::Array(items) => items.iter().find_map(|item| {
            item.get("errorCode")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }),
        _ => None,
    }
}

fn dpop_nonce_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("DPoP-Nonce")
        .or_else(|| headers.get("dpop-nonce"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn should_retry_with_dpop_nonce(
    status: reqwest::StatusCode,
    nonce: Option<&str>,
    body: &str,
) -> bool {
    let Some(nonce_value) = nonce else {
        return false;
    };
    if nonce_value.is_empty() {
        return false;
    }

    let lower_body = body.to_lowercase();
    status == reqwest::StatusCode::UNAUTHORIZED
        || lower_body.contains("use_dpop_nonce")
        || lower_body.contains("invalid_dpop_proof")
}

fn dpop_troubleshooting_hint(status: reqwest::StatusCode, body: &str) -> Option<&'static str> {
    if status != reqwest::StatusCode::UNAUTHORIZED {
        return None;
    }

    let lower_body = body.to_lowercase();
    if lower_body.contains("use_dpop_nonce")
        || lower_body.contains("invalid_dpop_proof")
        || lower_body.contains("\"dpop")
    {
        return Some("DPoP validation failed; verify your system clock is in sync and retry");
    }

    None
}

pub fn run_whoami_query(
    endpoint: &str,
    token: &str,
    person_id: &str,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Value> {
    execute_graphql(
        endpoint,
        token,
        WHOAMI_QUERY,
        &json!({ "id": person_id }),
        Some("WhoAmI"),
        GraphqlAccessContext::default(),
        dpop_options,
    )
}

pub fn fetch_login_2fa_state(
    endpoint: &str,
    token: &str,
    user_id: &str,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Login2faState> {
    let query = r#"
query Is2faOnLoginEnabled($input: Is2faOnLoginEnabledInput!) {
  is2faOnLoginEnabled(input: $input) {
    enabled
    hasApprovedSession
  }
}
"#;

    let data = execute_graphql(
        endpoint,
        token,
        query,
        &json!({ "input": { "userId": user_id } }),
        Some("Is2faOnLoginEnabled"),
        GraphqlAccessContext::default(),
        dpop_options,
    )?;

    let response = data.get("is2faOnLoginEnabled");
    let enabled = response
        .and_then(|v| v.get("enabled"))
        .and_then(Value::as_bool);
    let has_approved_session = response
        .and_then(|v| v.get("hasApprovedSession"))
        .and_then(Value::as_bool);

    Ok(Login2faState {
        enabled,
        has_approved_session,
    })
}

pub fn start_2fa_on_login(
    endpoint: &str,
    token: &str,
    user_id: &str,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Option<String>> {
    let mutation = r#"
mutation Start2faOnLogin($input: Start2faOnLoginInput!) {
  start2faOnLogin(input: $input) {
    mfaSessionId
  }
}
"#;

    let data = execute_graphql(
        endpoint,
        token,
        mutation,
        &json!({ "input": { "userId": user_id, "deviceName": "CLI", "deviceType": "CLI" } }),
        Some("Start2faOnLogin"),
        GraphqlAccessContext::default(),
        dpop_options,
    )?;

    let session_id = data
        .get("start2faOnLogin")
        .and_then(|v| v.get("mfaSessionId"))
        .and_then(Value::as_str)
        .map(ToString::to_string);

    Ok(session_id)
}

pub fn validate_2fa_on_login(
    endpoint: &str,
    token: &str,
    user_id: &str,
    mfa_session_id: &str,
    dpop_options: &DpopRuntimeOptions,
) -> Result<Option<String>> {
    let mutation = r#"
mutation Validate2faOnLogin($input: Validate2faOnLoginInput!) {
  validate2faOnLogin(input: $input) {
    status
  }
}
"#;

    let data = execute_graphql(
        endpoint,
        token,
        mutation,
        &json!({ "input": { "userId": user_id, "mfaSessionId": mfa_session_id } }),
        Some("Validate2faOnLogin"),
        GraphqlAccessContext::default(),
        dpop_options,
    )?;

    let status = data
        .get("validate2faOnLogin")
        .and_then(|v| v.get("status"))
        .and_then(Value::as_str)
        .map(ToString::to_string);

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DpopKeyBackend;
    use crate::dpop::DpopKeyMaterial;
    use mockito::Matcher;
    use mockito::Matcher::PartialJson;
    use mockito::Server;

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

    fn test_dpop_options() -> DpopRuntimeOptions {
        DpopRuntimeOptions {
            key_backend: DpopKeyBackend::File,
            pkcs11: None,
        }
    }

    fn ensure_runtime_dpop_key() {
        DpopKeyMaterial::load_or_create_for_options(&test_dpop_options())
            .expect("create runtime dpop key");
    }

    #[test]
    fn local_read_only_allows_queries() {
        let result = enforce_graphql_access_policy(
            "query BrokerOverview { ok }",
            Some("BrokerOverview"),
            GraphqlAccessContext::with_session_mode(Some(
                crate::session::SessionMode::LocalReadOnly,
            )),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn local_read_only_allows_explicit_allowlisted_mutations() {
        for operation_name in [
            "Start2faOnLogin",
            "Validate2faOnLogin",
            "revokeAuthAccessToken",
        ] {
            let result = enforce_graphql_access_policy(
                "mutation AllowedMutation { ok }",
                Some(operation_name),
                GraphqlAccessContext::with_session_mode(Some(
                    crate::session::SessionMode::LocalReadOnly,
                )),
            );

            assert!(result.is_ok(), "{operation_name} should be allowlisted");
        }
    }

    #[test]
    fn local_read_only_blocks_non_allowlisted_mutations() {
        let err = enforce_graphql_access_policy(
            "mutation BrokerAddToWatchlist { ok }",
            Some("BrokerAddToWatchlist"),
            GraphqlAccessContext::with_session_mode(Some(
                crate::session::SessionMode::LocalReadOnly,
            )),
        )
        .expect_err("mutation should be blocked");

        assert!(err.to_string().contains(LOCAL_READ_ONLY_ERROR_PREFIX));
        assert!(err.to_string().contains("BrokerAddToWatchlist"));
    }

    #[test]
    fn local_read_only_blocks_mutations_without_operation_name() {
        let err = enforce_graphql_access_policy(
            "mutation { ok }",
            None,
            GraphqlAccessContext::with_session_mode(Some(
                crate::session::SessionMode::LocalReadOnly,
            )),
        )
        .expect_err("unnamed mutation should be blocked");

        assert!(err.to_string().contains(LOCAL_READ_ONLY_ERROR_PREFIX));
        assert!(
            err.to_string()
                .contains("explicit allowlisted operation name")
        );
    }

    #[test]
    fn local_read_only_guard_runs_before_endpoint_validation_or_http() {
        let dpop = GraphqlDpopContext::with_key_material_for_tests(
            DpopKeyMaterial::from_private_scalar_bytes([3_u8; 32]).expect("fixed dpop key"),
        );
        let err = execute_graphql_with_headers_with_context(
            "not-a-valid-url",
            "token-1",
            "mutation BrokerAddToWatchlist { ok }",
            &json!({}),
            Some("BrokerAddToWatchlist"),
            GraphqlAccessContext::with_session_mode(Some(
                crate::session::SessionMode::LocalReadOnly,
            )),
            &[],
            &dpop,
        )
        .expect_err("local read-only should block before endpoint validation");

        assert!(err.to_string().contains(LOCAL_READ_ONLY_ERROR_PREFIX));
        assert!(!err.to_string().contains("Invalid GraphQL endpoint URL"));
    }

    #[test]
    fn execute_graphql_with_headers_includes_custom_header() {
        let mut server = Server::new();
        let mock = server
            .mock("POST", "/graphql")
            .match_header("authorization", "DPoP token-1")
            .match_header("dpop", Matcher::Regex(".+".to_string()))
            .match_header("x-sc-idempotency-id", "idem-123")
            .with_status(200)
            .with_body(r#"{"data":{"ok":true}}"#)
            .create();

        let dpop = GraphqlDpopContext::with_key_material_for_tests(
            DpopKeyMaterial::from_private_scalar_bytes([4_u8; 32]).expect("fixed dpop key"),
        );
        let result = execute_graphql_with_headers_with_context(
            &format!("{}/graphql", server.url()),
            "token-1",
            "query Q { ok }",
            &json!({}),
            Some("Q"),
            GraphqlAccessContext::default(),
            &[("x-sc-idempotency-id", "idem-123")],
            &dpop,
        )
        .expect("graphql call should succeed");

        assert_eq!(result["ok"], true);
        mock.assert();
    }

    #[test]
    fn execute_graphql_with_headers_uses_dpop_when_enabled() {
        let mut server = Server::new();
        let mock = server
            .mock("POST", "/graphql")
            .match_header("authorization", "DPoP token-1")
            .match_header("dpop", Matcher::Regex(".+".to_string()))
            .with_status(200)
            .with_body(r#"{"data":{"ok":true}}"#)
            .create();

        let dpop = GraphqlDpopContext::with_key_material_for_tests(
            DpopKeyMaterial::from_private_scalar_bytes([5_u8; 32]).expect("fixed dpop key"),
        );
        let result = execute_graphql_with_headers_with_context(
            &format!("{}/graphql", server.url()),
            "token-1",
            "query Q { ok }",
            &json!({}),
            Some("Q"),
            GraphqlAccessContext::default(),
            &[],
            &dpop,
        )
        .expect("graphql call should succeed");

        assert_eq!(result["ok"], true);
        mock.assert();
    }

    #[test]
    fn execute_graphql_with_headers_retries_once_on_dpop_nonce_challenge() {
        let mut server = Server::new();
        let first = server
            .mock("POST", "/graphql")
            .match_header("authorization", "DPoP token-1")
            .match_header("dpop", Matcher::Regex(".+".to_string()))
            .with_status(401)
            .with_header("DPoP-Nonce", "nonce-1")
            .with_body(r#"{"error":"use_dpop_nonce"}"#)
            .expect(1)
            .create();
        let second = server
            .mock("POST", "/graphql")
            .match_header("authorization", "DPoP token-1")
            .match_header("dpop", Matcher::Regex(".+".to_string()))
            .with_status(200)
            .with_body(r#"{"data":{"ok":true}}"#)
            .expect(1)
            .create();

        let dpop = GraphqlDpopContext::with_key_material_for_tests(
            DpopKeyMaterial::from_private_scalar_bytes([6_u8; 32]).expect("fixed dpop key"),
        );
        let result = execute_graphql_with_headers_with_context(
            &format!("{}/graphql", server.url()),
            "token-1",
            "query Q { ok }",
            &json!({}),
            Some("Q"),
            GraphqlAccessContext::default(),
            &[("x-sc-idempotency-id", "idem-123")],
            &dpop,
        )
        .expect("graphql call should succeed after nonce retry");

        assert_eq!(result["ok"], true);
        first.assert();
        second.assert();
    }

    #[test]
    fn execute_graphql_with_headers_adds_dpop_clock_hint_when_proof_is_rejected() {
        let mut server = Server::new();
        let first = server
            .mock("POST", "/graphql")
            .match_header("authorization", "DPoP token-1")
            .match_header("dpop", Matcher::Regex(".+".to_string()))
            .with_status(401)
            .with_body(r#"{"error":"invalid_dpop_proof"}"#)
            .expect(1)
            .create();

        let dpop = GraphqlDpopContext::with_key_material_for_tests(
            DpopKeyMaterial::from_private_scalar_bytes([7_u8; 32]).expect("fixed dpop key"),
        );
        let err = execute_graphql_with_headers_with_context(
            &format!("{}/graphql", server.url()),
            "token-1",
            "query Q { ok }",
            &json!({}),
            Some("Q"),
            GraphqlAccessContext::default(),
            &[],
            &dpop,
        )
        .expect_err("graphql call should fail with dpop troubleshooting hint");

        let msg = err.to_string();
        assert!(msg.contains("GraphQL HTTP error 401"));
        assert!(msg.contains("system clock is in sync"));
        first.assert();
    }

    #[test]
    fn execute_graphql_requires_existing_dpop_key_for_authenticated_session() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());

        let err = execute_graphql_with_headers(
            "https://example.com/graphql",
            "token-1",
            "query Q { ok }",
            &json!({}),
            Some("Q"),
            GraphqlAccessContext::default(),
            &[],
            &test_dpop_options(),
        )
        .expect_err("missing dpop key should fail before graphql request");

        assert_eq!(err.to_string(), DPOP_SESSION_KEY_RELOGIN_MESSAGE);
    }

    #[test]
    fn execute_graphql_http_error_does_not_echo_response_body() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let marker = "LEAK_SECRET_HTTP_1";
        let _mock = server
            .mock("POST", "/graphql")
            .with_status(500)
            .with_body(format!(
                r#"{{"error":"internal_error","access_token":"{marker}"}}"#
            ))
            .create();

        let err = execute_graphql_with_headers(
            &format!("{}/graphql", server.url()),
            "token-1",
            "query Q { ok }",
            &json!({}),
            Some("Q"),
            GraphqlAccessContext::default(),
            &[],
            &test_dpop_options(),
        )
        .expect_err("graphql call should fail");

        let msg = err.to_string();
        assert!(msg.contains("GraphQL HTTP error 500"));
        assert!(msg.contains("internal_error"));
        assert!(!msg.contains(marker));
    }

    #[test]
    fn execute_graphql_rate_limit_error_includes_retry_after_seconds() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let marker = "LEAK_SECRET_RATE_LIMIT_1";
        let _mock = server
            .mock("POST", "/graphql")
            .with_status(429)
            .with_header("Retry-After", "30")
            .with_body(format!(r#"rate limited {marker}"#))
            .create();

        let err = execute_graphql_with_headers(
            &format!("{}/graphql", server.url()),
            "token-1",
            "query Q { ok }",
            &json!({}),
            Some("BrokerOverview"),
            GraphqlAccessContext::default(),
            &[],
            &test_dpop_options(),
        )
        .expect_err("graphql call should fail");

        let msg = err.to_string();
        assert_eq!(
            msg,
            "RATE_LIMITED: backend rate limit exceeded during BrokerOverview; retry after 30s"
        );
        assert!(!msg.contains(marker));
        assert!(!msg.contains("GraphQL HTTP error 429"));
    }

    #[test]
    fn execute_graphql_rate_limit_error_falls_back_to_retry_later_without_valid_retry_after() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let marker = "LEAK_SECRET_RATE_LIMIT_2";
        let _mock = server
            .mock("POST", "/graphql")
            .with_status(429)
            .with_header("Retry-After", "Wed, 21 Oct 2015 07:28:00 GMT")
            .with_body(format!(
                r#"{{"error":"waf_rate_limit","token":"{marker}"}}"#
            ))
            .create();

        let err = execute_graphql_with_headers(
            &format!("{}/graphql", server.url()),
            "token-1",
            "query Q { ok }",
            &json!({}),
            None,
            GraphqlAccessContext::default(),
            &[],
            &test_dpop_options(),
        )
        .expect_err("graphql call should fail");

        let msg = err.to_string();
        assert_eq!(
            msg,
            "RATE_LIMITED: backend rate limit exceeded; retry later"
        );
        assert!(!msg.contains(marker));
    }

    #[test]
    fn execute_graphql_application_error_does_not_echo_response_body() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let marker = "LEAK_SECRET_GQL_2";
        let _mock = server
            .mock("POST", "/graphql")
            .with_status(200)
            .with_body(format!(
                r#"{{"errors":[{{"message":"token: {marker}","extensions":{{"code":"UNAUTHENTICATED"}}}}]}}"#
            ))
            .create();

        let err = execute_graphql_with_headers(
            &format!("{}/graphql", server.url()),
            "token-1",
            "query Q { ok }",
            &json!({}),
            Some("WhoAmI"),
            GraphqlAccessContext::default(),
            &[],
            &test_dpop_options(),
        )
        .expect_err("graphql call should fail");

        let msg = err.to_string();
        assert!(msg.contains("GraphQL returned errors"));
        assert!(msg.contains("code: UNAUTHENTICATED"));
        assert!(!msg.contains(marker));
    }

    #[test]
    fn graphql_application_error_message_maps_broker_quote_bad_user_input() {
        let message = graphql_application_error_message(
            Some("BrokerQuote"),
            &json!([{
                "message": "Invalid ISIN provided",
                "extensions": {
                    "code": "BAD_USER_INPUT"
                }
            }]),
        );

        assert_eq!(
            message,
            "Broker input invalid: field 'isin' must be a valid ISIN"
        );
    }

    #[test]
    fn graphql_application_error_message_preserves_generic_broker_quote_bad_user_input_without_isin_marker()
     {
        let message = graphql_application_error_message(
            Some("BrokerQuote"),
            &json!([{
                "message": "Some other quote validation failure",
                "extensions": {
                    "code": "BAD_USER_INPUT"
                }
            }]),
        );

        assert_eq!(
            message,
            "GraphQL returned errors for BrokerQuote (code: BAD_USER_INPUT)"
        );
    }

    #[test]
    fn graphql_application_error_message_maps_transaction_not_found_marker() {
        let message = graphql_application_error_message(
            Some("BrokerTransactionDetails"),
            &json!([{
                "message": "Transaction with ID tx-1 not found",
                "extensions": {
                    "code": "BAD_REQUEST"
                },
                "validationErrors": {
                    "errorCode": "TransactionNotFound"
                }
            }]),
        );

        assert_eq!(
            message,
            "Broker transaction not found: field 'transaction_id' was not found"
        );
    }

    #[test]
    fn graphql_application_error_message_preserves_generic_bad_request_for_other_broker_queries() {
        let message = graphql_application_error_message(
            Some("BrokerTransactions"),
            &json!([{
                "message": "Some other bad request",
                "extensions": {
                    "code": "BAD_REQUEST"
                }
            }]),
        );

        assert_eq!(
            message,
            "GraphQL returned errors for BrokerTransactions (code: BAD_REQUEST)"
        );
    }

    #[test]
    fn fetch_login_2fa_state_parses_enabled_and_approved_session() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let mock = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Is2faOnLoginEnabled",
                "variables": {
                    "input": {
                        "userId": "p-1"
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"data":{"is2faOnLoginEnabled":{"enabled":true,"hasApprovedSession":true}}}"#,
            )
            .expect(1)
            .create();

        let state = fetch_login_2fa_state(
            &format!("{}/graphql", server.url()),
            "token",
            "p-1",
            &test_dpop_options(),
        )
        .expect("2fa state should parse");

        assert_eq!(
            state,
            Login2faState {
                enabled: Some(true),
                has_approved_session: Some(true),
            }
        );
        mock.assert();
    }

    #[test]
    fn fetch_login_2fa_state_tolerates_missing_flags() {
        let _lock = crate::lock_test_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key();
        let mut server = Server::new();
        let mock = server
            .mock("POST", "/graphql")
            .match_body(PartialJson(serde_json::json!({
                "operation_name": "Is2faOnLoginEnabled",
                "variables": {
                    "input": {
                        "userId": "p-1"
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":{"is2faOnLoginEnabled":{}}}"#)
            .expect(1)
            .create();

        let state = fetch_login_2fa_state(
            &format!("{}/graphql", server.url()),
            "token",
            "p-1",
            &test_dpop_options(),
        )
        .expect("2fa state should parse");

        assert_eq!(
            state,
            Login2faState {
                enabled: None,
                has_approved_session: None,
            }
        );
        mock.assert();
    }
}
