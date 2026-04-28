use anyhow::Error;
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;

use crate::auth::REFRESH_RELOGIN_REQUIRED_PREFIX;
use crate::dpop::DPOP_SESSION_KEY_RELOGIN_MESSAGE;

#[derive(Debug, Serialize)]
struct MachineEnvelope {
    ok: bool,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<MachineError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hints: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MachineError {
    code: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedError {
    pub code: &'static str,
    pub exit_code: i32,
    pub hints: Vec<String>,
}

pub fn print_success(command: &str, data: Value) {
    let envelope = MachineEnvelope {
        ok: true,
        command: command.to_string(),
        data: Some(data),
        error: None,
        hints: Vec::new(),
    };
    println!(
        "{}",
        serde_json::to_string(&envelope).expect("Machine envelope should serialize")
    );
}

pub fn print_error(command: &str, err: &Error) -> i32 {
    let classified = classify_error(err);
    let envelope = MachineEnvelope {
        ok: false,
        command: command.to_string(),
        data: None,
        error: Some(MachineError {
            code: classified.code.to_string(),
            message: concise_error_message(&user_error_message(err)),
        }),
        hints: classified.hints.clone(),
    };
    println!(
        "{}",
        serde_json::to_string(&envelope).expect("Machine envelope should serialize")
    );
    classified.exit_code
}

pub fn user_error_message(err: &Error) -> String {
    sanitize_error_message(&full_error_text(err)).to_string()
}

pub fn human_error_message(err: &Error) -> String {
    sanitize_error_message(&format_error_chain_for_display(err)).to_string()
}

fn concise_error_message(message: &str) -> String {
    let sanitized = sanitize_error_message(message);
    let first_line = sanitized.lines().next().unwrap_or_default().trim();
    let max_chars = 280;
    if first_line.chars().count() <= max_chars {
        return first_line.to_string();
    }
    let truncated = first_line.chars().take(max_chars).collect::<String>();
    format!("{truncated}...")
}

fn sanitize_error_message(message: &str) -> Cow<'_, str> {
    if !message.contains(REFRESH_RELOGIN_REQUIRED_PREFIX) {
        return Cow::Borrowed(message);
    }

    let mut parts = message.split(REFRESH_RELOGIN_REQUIRED_PREFIX);
    let mut sanitized = parts.next().unwrap_or_default().to_string();
    for part in parts {
        sanitized.push_str(part.trim_start());
    }
    Cow::Owned(sanitized)
}

pub fn classify_error(err: &Error) -> ClassifiedError {
    let text = full_error_text(err);
    let lower = text.to_lowercase();

    if text.contains("Provide exactly one of --query or --query-file")
        || text.contains("Provide at most one of --variables or --variables-file")
        || text.contains("Token from stdin is empty")
    {
        return ClassifiedError {
            code: "invalid_input",
            exit_code: 10,
            hints: vec!["Check command arguments and input values.".to_string()],
        };
    }

    if lower.contains("installation code state") {
        return ClassifiedError {
            code: "installation_code_invalid_state",
            exit_code: 10,
            hints: vec![
                "Delete the local installation code file and rerun `sc installation-code`."
                    .to_string(),
            ],
        };
    }

    if lower.contains("unable to resolve broker portfolio id") {
        return ClassifiedError {
            code: "broker_context_missing",
            exit_code: 10,
            hints: vec![
                "Provide --portfolio-id or run `sc broker context select --portfolio-id ...`."
                    .to_string(),
            ],
        };
    }

    if lower.contains("broker input invalid:") {
        return ClassifiedError {
            code: "broker_input_invalid",
            exit_code: 10,
            hints: vec!["Check broker command inputs and retry.".to_string()],
        };
    }

    if text.contains("SAVINGS_PLAN_INPUT_INVALID:") {
        return ClassifiedError {
            code: "savings_plan_input_invalid",
            exit_code: 10,
            hints: vec!["Check savings-plan command inputs and retry.".to_string()],
        };
    }

    if text.contains("SAVINGS_PLAN_CONFIG_UNAVAILABLE:") {
        return ClassifiedError {
            code: "savings_plan_config_unavailable",
            exit_code: 10,
            hints: vec![
                "Savings plan configuration is unavailable for the instrument in this context."
                    .to_string(),
            ],
        };
    }

    if lower.contains("broker response invalid:") {
        return ClassifiedError {
            code: "broker_response_invalid",
            exit_code: 30,
            hints: vec![
                "Backend response shape did not match expected broker contract.".to_string(),
            ],
        };
    }

    if text.contains("TRADE_NOT_TRADABLE:") {
        return ClassifiedError {
            code: "trade_not_tradable",
            exit_code: 10,
            hints: vec![
                "Pick another instrument or venue and retry the pre-trade checks.".to_string(),
            ],
        };
    }

    if text.contains("CONFIRMATION_REQUIRED:") {
        return ClassifiedError {
            code: "confirmation_required",
            exit_code: 10,
            hints: vec![
                "Run phase 1 first, then repeat the same trade command with --confirm <id>."
                    .to_string(),
            ],
        };
    }

    if text.contains("CONFIRMATION_NOT_FOUND:") {
        return ClassifiedError {
            code: "confirmation_not_found",
            exit_code: 10,
            hints: vec!["Run phase 1 again to generate a fresh confirmation id.".to_string()],
        };
    }

    if text.contains("CONFIRMATION_EXPIRED:") {
        return ClassifiedError {
            code: "confirmation_expired",
            exit_code: 10,
            hints: vec![
                "Confirmation tokens expire quickly; rerun phase 1 and retry phase 2.".to_string(),
            ],
        };
    }

    if text.contains("CONFIRMATION_ALREADY_USED:") {
        return ClassifiedError {
            code: "confirmation_already_used",
            exit_code: 10,
            hints: vec!["Run phase 1 again to generate a new confirmation id.".to_string()],
        };
    }

    if text.contains("CONFIRMATION_ENV_MISMATCH:") {
        return ClassifiedError {
            code: "confirmation_env_mismatch",
            exit_code: 10,
            hints: vec![
                "Switch to the same env used in phase 1 or generate a new confirmation id."
                    .to_string(),
            ],
        };
    }

    if text.contains("CONFIRMATION_FIELDS_MISMATCH:") {
        return ClassifiedError {
            code: "confirmation_fields_mismatch",
            exit_code: 10,
            hints: vec![
                "Use exactly the phase-1 trade inputs (isin/amount-or-shares/venue/order-type/limit-price/stop-price), or rerun phase 1 if market data changed."
                    .to_string(),
            ],
        };
    }

    if text.contains("CONFIRMATION_WARNING_VERSION_REQUIRED:") {
        return ClassifiedError {
            code: "confirmation_warning_version_required",
            exit_code: 10,
            hints: vec!["Rerun phase 1 and confirm again; warning context changed.".to_string()],
        };
    }

    if text.contains("CONFIRMATION_UNSUITABLE_ACK_REQUIRED:") {
        return ClassifiedError {
            code: "confirmation_unsuitable_ack_required",
            exit_code: 10,
            hints: vec!["Repeat the phase-2 trade command with --accept-unsuitable.".to_string()],
        };
    }

    if text.contains("PRESENTATION_MAPPING_INCOMPLETE:") {
        return ClassifiedError {
            code: "presentation_mapping_incomplete",
            exit_code: 10,
            hints: vec![
                "Phase 1 presentation mapping is incomplete; retry or update the client."
                    .to_string(),
            ],
        };
    }

    if text.contains("APPROPRIATENESS_REQUIRED:") {
        return ClassifiedError {
            code: "appropriateness_required",
            exit_code: 10,
            hints: vec![
                "Complete appropriateness questionnaire flow before retrying trade.".to_string(),
            ],
        };
    }

    if text.contains("APPROPRIATENESS_WARNING_ACK_REQUIRED:") {
        return ClassifiedError {
            code: "appropriateness_warning_ack_required",
            exit_code: 10,
            hints: vec!["Provide the exact acknowledgement text when prompted.".to_string()],
        };
    }

    if text.contains("EX_ANTE_COST_UNAVAILABLE:") {
        return ClassifiedError {
            code: "ex_ante_cost_unavailable",
            exit_code: 10,
            hints: vec![
                "Quote or ex-ante costs are unavailable for this trade attempt.".to_string(),
            ],
        };
    }

    if text.contains("DISCLOSURE_NOT_ACKNOWLEDGED:") {
        return ClassifiedError {
            code: "disclosure_not_acknowledged",
            exit_code: 10,
            hints: vec![
                "Confirm the ex-ante disclosure acknowledgement exactly as prompted.".to_string(),
            ],
        };
    }

    if text.contains("ORDER_CONFIRMATION_REQUIRED:") {
        return ClassifiedError {
            code: "order_confirmation_required",
            exit_code: 10,
            hints: vec!["Provide exact PLACE ORDER confirmation when prompted.".to_string()],
        };
    }

    if text.contains("ORDER_SUBMISSION_FAILED:") {
        return ClassifiedError {
            code: "order_submission_failed",
            exit_code: 30,
            hints: vec![
                "Retry the command; idempotency key reuse protects against duplicate submit."
                    .to_string(),
            ],
        };
    }

    if lower.contains("trade input invalid:") {
        return ClassifiedError {
            code: "trade_input_invalid",
            exit_code: 10,
            hints: vec!["Check trade arguments and retry.".to_string()],
        };
    }

    if lower.contains("trade response invalid:") {
        return ClassifiedError {
            code: "trade_response_invalid",
            exit_code: 30,
            hints: vec!["Backend response shape did not match trade contract.".to_string()],
        };
    }

    if text.contains("No active session") {
        return ClassifiedError {
            code: "no_session",
            exit_code: 20,
            hints: vec!["Run 'sc login' first to create a session.".to_string()],
        };
    }

    if text.contains(REFRESH_RELOGIN_REQUIRED_PREFIX) {
        return ClassifiedError {
            code: "refresh_relogin_required",
            exit_code: 20,
            hints: vec!["Run 'sc login' again to create a fresh session.".to_string()],
        };
    }

    if text.contains(DPOP_SESSION_KEY_RELOGIN_MESSAGE) {
        return ClassifiedError {
            code: "refresh_relogin_required",
            exit_code: 20,
            hints: vec!["Run 'sc login' again to create a fresh session.".to_string()],
        };
    }

    if lower.contains("grant type")
        || lower.contains("unauthorized_client")
        || lower.contains("unsupported_grant_type")
    {
        return ClassifiedError {
            code: "auth_grant_not_enabled",
            exit_code: 20,
            hints: vec![
                "Enable required grants (Device Code, Refresh Token) for the OAuth app."
                    .to_string(),
            ],
        };
    }

    if lower.contains("failed to call graphql endpoint")
        || lower.contains("failed to fetch")
        || lower.contains("error sending request")
    {
        return ClassifiedError {
            code: "network_error",
            exit_code: 30,
            hints: vec!["Check network connectivity and endpoint reachability.".to_string()],
        };
    }

    if text.contains("RATE_LIMITED:") {
        let mut hints = vec!["Wait before retrying the command.".to_string()];
        if lower.contains("retry after") {
            hints.push("Respect backend Retry-After guidance when present.".to_string());
        }
        return ClassifiedError {
            code: "rate_limited",
            exit_code: 30,
            hints,
        };
    }

    if lower.contains("graphql http error") {
        return ClassifiedError {
            code: "backend_http_error",
            exit_code: 30,
            hints: vec!["Inspect backend response and token validity.".to_string()],
        };
    }

    ClassifiedError {
        code: "internal_error",
        exit_code: 1,
        hints: Vec::new(),
    }
}

fn full_error_text(err: &Error) -> String {
    err.chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

fn format_error_chain_for_display(err: &Error) -> String {
    let mut chain = err.chain().map(ToString::to_string);
    let Some(message) = chain.next() else {
        return String::new();
    };

    let causes = chain.collect::<Vec<_>>();
    if causes.is_empty() {
        return message;
    }

    let mut formatted = message;
    formatted.push_str("\n\nCaused by:");
    for cause in causes {
        formatted.push_str("\n  ");
        formatted.push_str(&cause);
    }
    formatted
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn classify_no_session_error() {
        let err = anyhow!("No active session for dev.");
        let c = classify_error(&err);
        assert_eq!(c.code, "no_session");
        assert_eq!(c.exit_code, 20);
        assert_eq!(c.hints, vec!["Run 'sc login' first to create a session."]);
    }

    #[test]
    fn classify_broker_context_missing_error() {
        let err = anyhow!("Unable to resolve broker portfolio id.");
        let c = classify_error(&err);
        assert_eq!(c.code, "broker_context_missing");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_broker_response_invalid_error() {
        let err =
            anyhow!("Broker response invalid: missing account.brokerPortfolio.watchlist.items");
        let c = classify_error(&err);
        assert_eq!(c.code, "broker_response_invalid");
        assert_eq!(c.exit_code, 30);
    }

    #[test]
    fn classify_installation_code_invalid_state_error() {
        let err = anyhow!(
            "Invalid installation code state at /tmp/sc-installation_code.json: invalid JSON. Delete /tmp/sc-installation_code.json and rerun `sc installation-code`."
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "installation_code_invalid_state");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_refresh_relogin_required_error() {
        let err = anyhow!(
            "{REFRESH_RELOGIN_REQUIRED_PREFIX} Token refresh requires a new login (OAuth 400 - invalid_grant). Run 'sc login'."
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "refresh_relogin_required");
        assert_eq!(c.exit_code, 20);
    }

    #[test]
    fn classify_dpop_session_key_relogin_error() {
        let err = anyhow!(crate::dpop::DPOP_SESSION_KEY_RELOGIN_MESSAGE);
        let c = classify_error(&err);
        assert_eq!(c.code, "refresh_relogin_required");
        assert_eq!(c.exit_code, 20);
        assert_eq!(
            c.hints,
            vec!["Run 'sc login' again to create a fresh session."]
        );
    }

    #[test]
    fn classify_rate_limited_error_without_retry_after() {
        let err =
            anyhow!("RATE_LIMITED: backend rate limit exceeded during BrokerOverview; retry later");
        let c = classify_error(&err);
        assert_eq!(c.code, "rate_limited");
        assert_eq!(c.exit_code, 30);
        assert_eq!(c.hints, vec!["Wait before retrying the command."]);
    }

    #[test]
    fn classify_rate_limited_error_with_retry_after_guidance() {
        let err = anyhow!(
            "RATE_LIMITED: backend rate limit exceeded during BrokerOverview; retry after 30s"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "rate_limited");
        assert_eq!(c.exit_code, 30);
        assert_eq!(
            c.hints,
            vec![
                "Wait before retrying the command.",
                "Respect backend Retry-After guidance when present."
            ]
        );
    }

    #[test]
    fn classify_savings_plan_input_invalid_error() {
        let err = anyhow!("SAVINGS_PLAN_INPUT_INVALID: field 'amount' must be a positive decimal");
        let c = classify_error(&err);
        assert_eq!(c.code, "savings_plan_input_invalid");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_savings_plan_config_unavailable_error() {
        let err = anyhow!("SAVINGS_PLAN_CONFIG_UNAVAILABLE: missing schedules in config");
        let c = classify_error(&err);
        assert_eq!(c.code, "savings_plan_config_unavailable");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_trade_not_tradable_error() {
        let err = anyhow!("TRADE_NOT_TRADABLE: buy trading is not available on venue 'MUNC'");
        let c = classify_error(&err);
        assert_eq!(c.code, "trade_not_tradable");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_disclosure_not_acknowledged_error() {
        let err =
            anyhow!("DISCLOSURE_NOT_ACKNOWLEDGED: ex-ante disclosure acknowledgement required");
        let c = classify_error(&err);
        assert_eq!(c.code, "disclosure_not_acknowledged");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_order_submission_failed_error() {
        let err = anyhow!("ORDER_SUBMISSION_FAILED: timeout");
        let c = classify_error(&err);
        assert_eq!(c.code, "order_submission_failed");
        assert_eq!(c.exit_code, 30);
    }

    #[test]
    fn classify_confirmation_unsuitable_ack_required_error() {
        let err = anyhow!(
            "CONFIRMATION_UNSUITABLE_ACK_REQUIRED: phase 1 marked this instrument as not suitable"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "confirmation_unsuitable_ack_required");
        assert_eq!(c.exit_code, 10);
        assert!(
            c.hints
                .iter()
                .any(|hint| hint.contains("--accept-unsuitable"))
        );
    }

    #[test]
    fn classify_presentation_mapping_incomplete_error() {
        let err =
            anyhow!("PRESENTATION_MAPPING_INCOMPLETE: missing required path '/result/intent'");
        let c = classify_error(&err);
        assert_eq!(c.code, "presentation_mapping_incomplete");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn concise_error_message_uses_first_line_only() {
        let text = "first line\nsecond line";
        assert_eq!(concise_error_message(text), "first line");
    }

    #[test]
    fn human_error_message_formats_causes_on_separate_lines() {
        let err = anyhow!("inner failure").context("outer context.");

        assert_eq!(
            human_error_message(&err),
            "outer context.\n\nCaused by:\n  inner failure"
        );
    }

    #[test]
    fn human_error_message_formats_multiple_causes_in_order() {
        let err = anyhow!("module load failed")
            .context("Failed to load PKCS#11 module")
            .context("Failed to load PKCS#11 DPoP key material")
            .context(crate::dpop::DPOP_SESSION_KEY_RELOGIN_MESSAGE);

        assert_eq!(
            human_error_message(&err),
            "The DPoP signing key for the current session is missing or changed; run 'sc login' again.\n\nCaused by:\n  Failed to load PKCS#11 DPoP key material\n  Failed to load PKCS#11 module\n  module load failed"
        );
    }

    #[test]
    fn human_error_message_does_not_join_sentence_context_with_colon() {
        let err = anyhow!("inner failure").context(
            "The DPoP signing key for the current session is missing or changed; run 'sc login' again.",
        );

        let message = human_error_message(&err);

        assert!(!message.contains(".:"));
        assert_eq!(
            message,
            "The DPoP signing key for the current session is missing or changed; run 'sc login' again.\n\nCaused by:\n  inner failure"
        );
    }

    #[test]
    fn user_error_message_remains_single_line_for_embedding() {
        let err = anyhow!("inner failure").context("outer context");

        assert_eq!(user_error_message(&err), "outer context: inner failure");
    }

    #[test]
    fn concise_error_message_truncates_long_lines() {
        let text = "x".repeat(400);
        let out = concise_error_message(&text);
        assert!(out.len() < 400);
        assert!(out.ends_with("..."));
    }

    #[test]
    fn concise_error_message_strips_refresh_relogin_prefix() {
        let text = format!("{REFRESH_RELOGIN_REQUIRED_PREFIX} Token refresh requires a new login.");
        assert_eq!(
            concise_error_message(&text),
            "Token refresh requires a new login."
        );
    }

    #[test]
    fn user_error_message_strips_refresh_relogin_prefix_from_contextualized_errors() {
        let err = anyhow!(
            "Token refresh after unauthorized response failed: {REFRESH_RELOGIN_REQUIRED_PREFIX} Token refresh requires a new login."
        );
        assert_eq!(
            user_error_message(&err),
            "Token refresh after unauthorized response failed: Token refresh requires a new login."
        );
    }

    #[test]
    fn concise_error_message_strips_refresh_relogin_prefix_from_contextualized_errors() {
        let text = format!(
            "Token refresh after unauthorized response failed: {REFRESH_RELOGIN_REQUIRED_PREFIX} Token refresh requires a new login."
        );
        assert_eq!(
            concise_error_message(&text),
            "Token refresh after unauthorized response failed: Token refresh requires a new login."
        );
    }

    #[test]
    fn classify_refresh_relogin_required_error_from_error_chain() {
        let err = anyhow!("Token refresh requires a new login.")
            .context(format!("{REFRESH_RELOGIN_REQUIRED_PREFIX} refresh marker"));
        assert_eq!(classify_error(&err).code, "refresh_relogin_required");
    }
}
