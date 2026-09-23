use anyhow::Error;
use clap::error::{Error as ClapError, ErrorKind};
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;

use crate::auth::REFRESH_RELOGIN_REQUIRED_PREFIX;
use crate::broker_shared::{
    BROKER_PORTFOLIO_GROUP_ALREADY_EXISTS_ERROR_PREFIX,
    BROKER_PORTFOLIO_GROUP_CANNOT_BE_REMOVED_ERROR_PREFIX,
    BROKER_PORTFOLIO_GROUP_INVALID_CHARACTERS_ERROR_PREFIX,
    BROKER_PORTFOLIO_GROUP_NOT_FOUND_ERROR_PREFIX,
    BROKER_PORTFOLIO_GROUPS_LIMIT_REACHED_ERROR_PREFIX,
    BROKER_PORTFOLIO_GROUPS_QUOTA_EXCEEDED_ERROR_PREFIX,
};
use crate::dpop::{DPOP_SESSION_KEY_RELOGIN_MESSAGE, SecureEnclaveKeyLocked};
use crate::graphql::{BROKER_TRANSACTION_NOT_FOUND_ERROR_PREFIX, LOCAL_READ_ONLY_ERROR_PREFIX};
use crate::session::SessionStorageError;
use crate::trade_controls::{
    LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX, LOCAL_TRADE_CONTROL_ISIN_NOT_ALLOWED_PREFIX,
    LOCAL_TRADE_CONTROL_ORDER_NOTIONAL_EXCEEDED_PREFIX,
};

const DEVICE_LOCKED_ERROR_CODE: &str = "device_locked";
const DEVICE_LOCKED_ERROR_MESSAGE: &str = "The Mac is locked, so the Secure Enclave signing key cannot be used. Unlock the Mac and retry.";
const DEVICE_LOCKED_ERROR_HINT: &str =
    "If the Mac is already unlocked, check Secure Enclave and keychain access.";

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
    print_classified_error(command, &user_error_message(err), &classified)
}

pub fn print_clap_error(command: &str, err: &ClapError) -> i32 {
    let classified = classify_clap_error(err);
    print_classified_error(command, &err.to_string(), &classified)
}

fn print_classified_error(command: &str, message: &str, classified: &ClassifiedError) -> i32 {
    let envelope = classified_error_envelope(command, message, classified);
    println!(
        "{}",
        serde_json::to_string(&envelope).expect("Machine envelope should serialize")
    );
    classified.exit_code
}

fn classified_error_envelope(
    command: &str,
    message: &str,
    classified: &ClassifiedError,
) -> MachineEnvelope {
    MachineEnvelope {
        ok: false,
        command: command.to_string(),
        data: None,
        error: Some(MachineError {
            code: classified.code.to_string(),
            message: machine_error_message(message, classified),
        }),
        hints: classified.hints.clone(),
    }
}

fn machine_error_message(message: &str, classified: &ClassifiedError) -> String {
    if classified.code == DEVICE_LOCKED_ERROR_CODE {
        return DEVICE_LOCKED_ERROR_MESSAGE.to_string();
    }

    concise_error_message(message)
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
    if !message.contains(REFRESH_RELOGIN_REQUIRED_PREFIX)
        && !message.contains(LOCAL_READ_ONLY_ERROR_PREFIX)
        && !message.contains(LOCAL_TRADE_CONTROL_ISIN_NOT_ALLOWED_PREFIX)
        && !message.contains(LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX)
        && !message.contains(LOCAL_TRADE_CONTROL_ORDER_NOTIONAL_EXCEEDED_PREFIX)
    {
        return Cow::Borrowed(message);
    }

    let mut sanitized = message.to_string();
    for prefix in [
        REFRESH_RELOGIN_REQUIRED_PREFIX,
        LOCAL_READ_ONLY_ERROR_PREFIX,
        LOCAL_TRADE_CONTROL_ISIN_NOT_ALLOWED_PREFIX,
        LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX,
        LOCAL_TRADE_CONTROL_ORDER_NOTIONAL_EXCEEDED_PREFIX,
    ] {
        let mut parts = sanitized.split(prefix);
        let mut rebuilt = parts.next().unwrap_or_default().to_string();
        for part in parts {
            rebuilt.push_str(part.trim_start());
        }
        sanitized = rebuilt;
    }
    Cow::Owned(sanitized)
}

fn looks_like_clap_input_error(lower: &str) -> bool {
    [
        "required arguments were not provided",
        "unexpected argument",
        "unexpected value",
        "unrecognized subcommand",
        "invalid value",
        "a value is required for",
        "one of the values isn't valid",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn classify_clap_error(err: &ClapError) -> ClassifiedError {
    match err.kind() {
        ErrorKind::ArgumentConflict
        | ErrorKind::NoEquals
        | ErrorKind::InvalidValue
        | ErrorKind::InvalidSubcommand
        | ErrorKind::MissingRequiredArgument
        | ErrorKind::MissingSubcommand
        | ErrorKind::InvalidUtf8
        | ErrorKind::TooManyValues
        | ErrorKind::TooFewValues
        | ErrorKind::ValueValidation
        | ErrorKind::WrongNumberOfValues
        | ErrorKind::UnknownArgument => ClassifiedError {
            code: "invalid_input",
            exit_code: 10,
            hints: vec!["Check command arguments and input values.".to_string()],
        },
        _ => ClassifiedError {
            code: "internal_error",
            exit_code: 1,
            hints: Vec::new(),
        },
    }
}

pub fn classify_error(err: &Error) -> ClassifiedError {
    if let Some(storage_error) = session_storage_error_in_chain(err) {
        return match storage_error {
            SessionStorageError::SecretStoreUnavailable { .. } => ClassifiedError {
                code: "secret_storage_unavailable",
                exit_code: 20,
                hints: vec![
                    "Set `session_backend = \"file\"` under `[auth]` in the scalable-cli config.toml, then run 'sc login' again.".to_string(),
                ],
            },
        };
    }

    if secure_enclave_key_locked_in_chain(err) {
        return ClassifiedError {
            code: DEVICE_LOCKED_ERROR_CODE,
            exit_code: 20,
            hints: vec![DEVICE_LOCKED_ERROR_HINT.to_string()],
        };
    }

    let text = full_error_text(err);
    let lower = text.to_lowercase();

    if text.contains(crate::fixed_term_projections::NOT_FOUND) {
        return ClassifiedError {
            code: "fixed_term_account_not_found",
            exit_code: 10,
            hints: vec!["Run `sc fixed-term list` and select an available account id.".into()],
        };
    }
    if lower.contains("fixed-term response invalid:") {
        return ClassifiedError {
            code: "fixed_term_response_invalid",
            exit_code: 30,
            hints: vec!["Backend response did not match the Fixed-Term contract.".into()],
        };
    }

    if text.contains("Provide exactly one of --query or --query-file")
        || text.contains("Provide at most one of --variables or --variables-file")
        || text.contains("Token from stdin is empty")
        || looks_like_clap_input_error(&lower)
    {
        return ClassifiedError {
            code: "invalid_input",
            exit_code: 10,
            hints: vec!["Check command arguments and input values.".to_string()],
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

    if lower.contains("unable to resolve overnight savings account id") {
        return ClassifiedError {
            code: "overnight_selection_required",
            exit_code: 10,
            hints: vec![
                "If multiple overnight accounts exist, rerun with `--savings-account-id <ID>`."
                    .to_string(),
            ],
        };
    }

    if lower.contains("overnight input invalid:") {
        return ClassifiedError {
            code: "overnight_input_invalid",
            exit_code: 10,
            hints: vec!["Check overnight command inputs and retry.".to_string()],
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

    if text.contains("SAVINGS_PLAN_EX_ANTE_COST_UNAVAILABLE:") {
        return ClassifiedError {
            code: "savings_plan_ex_ante_cost_unavailable",
            exit_code: 30,
            hints: vec![
                "Ex-ante costs are unavailable. Do not submit; rerun phase 1 later.".to_string(),
            ],
        };
    }

    if text.contains("SAVINGS_PLAN_CONFIRMATION_NOT_FOUND:") {
        return ClassifiedError {
            code: "savings_plan_confirmation_not_found",
            exit_code: 10,
            hints: vec![
                "Run savings-plan phase 1 again to obtain a new confirmation id.".to_string(),
            ],
        };
    }

    if text.contains("SAVINGS_PLAN_CONFIRMATION_EXPIRED:") {
        return ClassifiedError {
            code: "savings_plan_confirmation_expired",
            exit_code: 10,
            hints: vec!["Run savings-plan phase 1 again; the confirmation expired.".to_string()],
        };
    }

    if text.contains("SAVINGS_PLAN_CONFIRMATION_ALREADY_USED:") {
        return ClassifiedError {
            code: "savings_plan_confirmation_already_used",
            exit_code: 10,
            hints: vec![
                "Run a new savings-plan phase 1 preview for a new client instruction.".to_string(),
            ],
        };
    }

    if text.contains("SAVINGS_PLAN_CONFIRMATION_CORRUPT:") {
        return ClassifiedError {
            code: "savings_plan_confirmation_corrupt",
            exit_code: 20,
            hints: vec![
                "Do not submit. Clear the corrupt local confirmation state and run phase 1 again."
                    .to_string(),
            ],
        };
    }

    for (prefix, code) in [
        (
            "SAVINGS_PLAN_CONFIRMATION_ENV_MISMATCH:",
            "savings_plan_confirmation_env_mismatch",
        ),
        (
            "SAVINGS_PLAN_CONFIRMATION_ACCOUNT_MISMATCH:",
            "savings_plan_confirmation_account_mismatch",
        ),
        (
            "SAVINGS_PLAN_CONFIRMATION_PORTFOLIO_MISMATCH:",
            "savings_plan_confirmation_portfolio_mismatch",
        ),
        (
            "SAVINGS_PLAN_CONFIRMATION_FIELDS_MISMATCH:",
            "savings_plan_confirmation_fields_mismatch",
        ),
    ] {
        if text.contains(prefix) {
            return ClassifiedError {
                code,
                exit_code: 10,
                hints: vec![
                    "The preview no longer matches. Run savings-plan phase 1 again.".to_string(),
                ],
            };
        }
    }

    if text.contains("SAVINGS_PLAN_SUBMISSION_FAILED:") {
        return ClassifiedError {
            code: "savings_plan_submission_failed",
            exit_code: 30,
            hints: vec!["The backend rejected the submission. Run a new phase 1 preview before trying again.".to_string()],
        };
    }

    if text.contains("SAVINGS_PLAN_SUBMISSION_UNKNOWN:") {
        return ClassifiedError {
            code: "savings_plan_submission_unknown",
            exit_code: 30,
            hints: vec!["Do not retry the add command. Inspect `sc broker savings-plans` for the same account and portfolio first.".to_string()],
        };
    }

    if lower.contains("broker input invalid:") {
        return ClassifiedError {
            code: "broker_input_invalid",
            exit_code: 10,
            hints: vec!["Check broker command inputs and retry.".to_string()],
        };
    }

    if text.contains(BROKER_TRANSACTION_NOT_FOUND_ERROR_PREFIX) {
        return ClassifiedError {
            code: "broker_transaction_not_found",
            exit_code: 10,
            hints: vec![
                "Check the transaction id and selected broker portfolio, then retry.".to_string(),
            ],
        };
    }

    if text.contains(BROKER_PORTFOLIO_GROUP_NOT_FOUND_ERROR_PREFIX) {
        return ClassifiedError {
            code: "portfolio_group_not_found",
            exit_code: 10,
            hints: vec![
                "Check the group id and selected broker portfolio, then retry.".to_string(),
            ],
        };
    }

    if text.contains(BROKER_PORTFOLIO_GROUP_INVALID_CHARACTERS_ERROR_PREFIX) {
        return ClassifiedError {
            code: "portfolio_group_invalid_characters",
            exit_code: 10,
            hints: vec!["Remove the unsupported characters and retry.".to_string()],
        };
    }

    if text.contains(BROKER_PORTFOLIO_GROUPS_QUOTA_EXCEEDED_ERROR_PREFIX) {
        return ClassifiedError {
            code: "portfolio_groups_quota_exceeded",
            exit_code: 10,
            hints: vec![
                "Upgrade the offer or reduce existing portfolio groups, then retry.".to_string(),
            ],
        };
    }

    if text.contains(BROKER_PORTFOLIO_GROUPS_LIMIT_REACHED_ERROR_PREFIX) {
        return ClassifiedError {
            code: "portfolio_groups_limit_reached",
            exit_code: 10,
            hints: vec![
                "The technical maximum number of portfolio groups has been reached.".to_string(),
            ],
        };
    }

    if text.contains(BROKER_PORTFOLIO_GROUP_ALREADY_EXISTS_ERROR_PREFIX) {
        return ClassifiedError {
            code: "portfolio_group_already_exists",
            exit_code: 10,
            hints: vec!["Choose a different portfolio group name and retry.".to_string()],
        };
    }

    if text.contains(BROKER_PORTFOLIO_GROUP_CANNOT_BE_REMOVED_ERROR_PREFIX) {
        return ClassifiedError {
            code: "portfolio_group_cannot_be_removed",
            exit_code: 10,
            hints: vec!["Remove dependent items or pending activity, then retry.".to_string()],
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

    if lower.contains("overnight response invalid:") {
        return ClassifiedError {
            code: "overnight_response_invalid",
            exit_code: 30,
            hints: vec![
                "Backend response shape did not match expected overnight contract.".to_string(),
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

    if text.contains(LOCAL_TRADE_CONTROL_ISIN_NOT_ALLOWED_PREFIX) {
        return ClassifiedError {
            code: "trade_control_isin_not_allowed",
            exit_code: 10,
            hints: vec![
                "Choose an ISIN listed in local trade_controls.allowed_isins or update the local config."
                    .to_string(),
            ],
        };
    }

    if text.contains(LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX) {
        return ClassifiedError {
            code: "trade_control_isin_denied",
            exit_code: 10,
            hints: vec![
                "Choose another ISIN or update local trade_controls.denied_isins.".to_string(),
            ],
        };
    }

    if text.contains(LOCAL_TRADE_CONTROL_ORDER_NOTIONAL_EXCEEDED_PREFIX) {
        return ClassifiedError {
            code: "trade_control_order_notional_exceeded",
            exit_code: 10,
            hints: vec![
                "Reduce the order size or raise local trade_controls.max_order_notional, then rerun phase 1."
                    .to_string(),
            ],
        };
    }

    if text.contains(LOCAL_READ_ONLY_ERROR_PREFIX) {
        return ClassifiedError {
            code: "local_read_only",
            exit_code: 10,
            hints: vec![
                "Run `sc login` without `--local-read-only` to perform write operations."
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

    if text.contains("CONFIRMATION_APPROPRIATENESS_WARNING_ACK_REQUIRED:") {
        return ClassifiedError {
            code: "confirmation_appropriateness_warning_ack_required",
            exit_code: 10,
            hints: vec![
                "Repeat the phase-2 trade command with --acknowledge-appropriateness-warning."
                    .to_string(),
            ],
        };
    }

    if text.contains("SUITABILITY_QUESTIONNAIRE_REQUIRED:") {
        return ClassifiedError {
            code: "suitability_questionnaire_required",
            exit_code: 10,
            hints: vec![
                "Complete the required suitability questionnaire in web or mobile, then rerun phase 1."
                    .to_string(),
            ],
        };
    }

    if text.contains("UNSUPPORTED_TRADE_SUITABILITY_TYPE:") {
        return ClassifiedError {
            code: "unsupported_trade_suitability_type",
            exit_code: 10,
            hints: vec!["Use web or mobile for this product flow.".to_string()],
        };
    }

    if text.contains("SUITABILITY_ID_MISSING:") {
        return ClassifiedError {
            code: "suitability_id_missing",
            exit_code: 30,
            hints: vec![
                "The backend omitted the suitability id needed for order submission; retry later or investigate the response contract."
                    .to_string(),
            ],
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
                "Check the error details before retrying; if the outcome may be ambiguous, check transactions or order status first. Idempotency key reuse still protects an intentional retry."
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

fn session_storage_error_in_chain(err: &Error) -> Option<&SessionStorageError> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<SessionStorageError>())
}

fn secure_enclave_key_locked_in_chain(err: &Error) -> bool {
    err.downcast_ref::<SecureEnclaveKeyLocked>().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn clap_error(kind: ErrorKind) -> ClapError {
        ClapError::raw(kind, "parse failed")
    }

    fn locked_secure_enclave_error() -> Error {
        crate::dpop::map_secure_enclave_signing_error(-25308, "OSStatus error -25308")
            .context("Failed generating DPoP proof for GraphQL request")
    }

    #[test]
    fn classify_locked_secure_enclave_key_error() {
        let err = locked_secure_enclave_error();

        let classified = classify_error(&err);

        assert_eq!(classified.code, "device_locked");
        assert_eq!(classified.exit_code, 20);
        assert_eq!(
            classified.hints,
            vec!["If the Mac is already unlocked, check Secure Enclave and keychain access."]
        );
    }

    #[test]
    fn locked_secure_enclave_key_uses_fixed_machine_envelope_message() {
        let err = locked_secure_enclave_error();
        let classified = classify_error(&err);
        let envelope =
            classified_error_envelope("broker.quote", &user_error_message(&err), &classified);
        let value = serde_json::to_value(envelope).expect("serialize machine envelope");

        assert_eq!(value["ok"], false);
        assert_eq!(value["command"], "broker.quote");
        assert_eq!(value["error"]["code"], "device_locked");
        assert_eq!(
            value["error"]["message"],
            "The Mac is locked, so the Secure Enclave signing key cannot be used. Unlock the Mac and retry."
        );
        assert_eq!(
            value["hints"],
            serde_json::json!([
                "If the Mac is already unlocked, check Secure Enclave and keychain access."
            ])
        );
        assert!(
            !value["error"]["message"]
                .as_str()
                .expect("machine error message")
                .contains("OSStatus")
        );
    }

    #[test]
    fn generic_osstatus_interaction_not_allowed_text_remains_internal_error() {
        let err =
            anyhow!("Failed generating DPoP proof for GraphQL request: OSStatus error -25308");

        let classified = classify_error(&err);

        assert_eq!(classified.code, "internal_error");
        assert_eq!(classified.exit_code, 1);
    }

    #[test]
    fn unmarked_secure_enclave_error_remains_internal_error() {
        let err = crate::dpop::map_secure_enclave_signing_error(
            -25309,
            "native Secure Enclave signing failure",
        )
        .context("Failed generating DPoP proof for GraphQL request");

        let classified = classify_error(&err);

        assert_eq!(classified.code, "internal_error");
        assert_eq!(classified.exit_code, 1);
    }

    #[test]
    fn human_error_message_retains_locked_secure_enclave_guidance() {
        let message = human_error_message(&locked_secure_enclave_error());

        assert!(message.contains(
            "The Mac is locked, so the Secure Enclave signing key cannot be used. Unlock the Mac and retry."
        ));
    }

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
    fn portfolio_group_membership_validation_uses_machine_input_error_envelope() {
        let err = anyhow!(
            "Broker input invalid: ISINs already assigned to portfolio group 'group-1': [US0378331005]"
        );
        let classified = classify_error(&err);
        let envelope = classified_error_envelope(
            "broker.portfolio-groups.assign",
            &user_error_message(&err),
            &classified,
        );
        let value = serde_json::to_value(envelope).expect("serialize machine envelope");

        assert_eq!(value["command"], "broker.portfolio-groups.assign");
        assert_eq!(value["error"]["code"], "broker_input_invalid");
        assert_eq!(
            value["error"]["message"],
            "Broker input invalid: ISINs already assigned to portfolio group 'group-1': [US0378331005]"
        );
    }

    #[test]
    fn classify_clap_missing_required_argument_error() {
        let err = anyhow!(
            "error: the following required arguments were not provided:\n  --portfolio-id <PORTFOLIO_ID>\n\nUsage: sc broker context select --portfolio-id <PORTFOLIO_ID>"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "invalid_input");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_clap_invalid_subcommand_error() {
        let err = anyhow!("error: unrecognized subcommand 'bogus'\n\nUsage: sc broker <COMMAND>");
        let c = classify_error(&err);
        assert_eq!(c.code, "invalid_input");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_non_clap_subcommand_text_does_not_match_invalid_input() {
        let err = anyhow!("downstream service reported that the subcommand failed unexpectedly");
        let c = classify_error(&err);
        assert_eq!(c.code, "internal_error");
        assert_eq!(c.exit_code, 1);
    }

    #[test]
    fn classify_common_clap_kinds_as_invalid_input() {
        for kind in [
            ErrorKind::UnknownArgument,
            ErrorKind::InvalidValue,
            ErrorKind::TooManyValues,
            ErrorKind::TooFewValues,
            ErrorKind::WrongNumberOfValues,
            ErrorKind::ArgumentConflict,
            ErrorKind::NoEquals,
            ErrorKind::ValueValidation,
        ] {
            let classified = classify_clap_error(&clap_error(kind));
            assert_eq!(classified.code, "invalid_input");
            assert_eq!(classified.exit_code, 10);
            assert_eq!(
                classified.hints,
                vec!["Check command arguments and input values."]
            );
        }
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
    fn classify_broker_transaction_not_found_error() {
        let err = anyhow!("Broker transaction not found: field 'transaction_id' was not found");
        let c = classify_error(&err);
        assert_eq!(c.code, "broker_transaction_not_found");
        assert_eq!(c.exit_code, 10);
        assert!(
            c.hints
                .iter()
                .any(|hint| hint.contains("selected broker portfolio"))
        );
    }

    #[test]
    fn classify_portfolio_group_not_found_error() {
        let err = anyhow!(
            "Broker portfolio group not found: portfolio group 'group-1' was not found in the active portfolio"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "portfolio_group_not_found");
        assert_eq!(c.exit_code, 10);
        assert!(
            c.hints
                .iter()
                .any(|hint| hint.contains("selected broker portfolio"))
        );
    }

    #[test]
    fn classify_portfolio_group_lifecycle_validation_errors() {
        let cases = [
            (
                "Broker portfolio group invalid characters: name contains '@'",
                "portfolio_group_invalid_characters",
            ),
            (
                "Broker portfolio groups quota exceeded: offer does not allow another group",
                "portfolio_groups_quota_exceeded",
            ),
            (
                "Broker portfolio groups limit reached: technical maximum reached",
                "portfolio_groups_limit_reached",
            ),
            (
                "Broker portfolio group already exists: Group already exists",
                "portfolio_group_already_exists",
            ),
            (
                "Broker portfolio group cannot be removed: The portfolio group cannot be removed",
                "portfolio_group_cannot_be_removed",
            ),
        ];

        for (message, code) in cases {
            assert_eq!(classify_error(&anyhow!(message)).code, code);
        }
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
    fn classify_local_read_only_error() {
        let err = anyhow!(
            "{LOCAL_READ_ONLY_ERROR_PREFIX} local read-only mode blocks write operation 'BrokerAddToWatchlist'. Re-login without --local-read-only to perform write operations."
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "local_read_only");
        assert_eq!(c.exit_code, 10);
        assert_eq!(
            c.hints,
            vec!["Run `sc login` without `--local-read-only` to perform write operations."]
        );
    }

    #[test]
    fn classify_trade_control_isin_not_allowed_error() {
        let err = anyhow!(
            "{LOCAL_TRADE_CONTROL_ISIN_NOT_ALLOWED_PREFIX} local trade controls require ISIN 'US0378331005' to be present in allowed_isins"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "trade_control_isin_not_allowed");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_trade_control_isin_denied_error() {
        let err = anyhow!(
            "{LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX} local trade controls deny ISIN 'US0378331005' via denied_isins"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "trade_control_isin_denied");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_trade_control_order_notional_exceeded_error() {
        let err = anyhow!(
            "{LOCAL_TRADE_CONTROL_ORDER_NOTIONAL_EXCEEDED_PREFIX} local trade controls block estimated order notional '1200' because it exceeds max_order_notional '1000'"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "trade_control_order_notional_exceeded");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_secret_storage_unavailable_error() {
        let err = anyhow::Error::new(SessionStorageError::secret_store_unavailable(
            keyring_core::Error::PlatformFailure(Box::new(std::io::Error::other(
                "DBus error: org.freedesktop.secrets",
            ))),
        ))
        .context("outer context");
        let c = classify_error(&err);
        assert_eq!(c.code, "secret_storage_unavailable");
        assert_eq!(c.exit_code, 20);
        assert!(c.hints[0].contains("session_backend = \"file\""));
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
    fn classify_wrapped_savings_plan_input_invalid_error() {
        let err = anyhow!(
            "SAVINGS_PLAN_INPUT_INVALID: Broker input invalid: field 'isin' must be a valid ISIN"
        );
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
    fn classify_new_savings_plan_confirmation_errors() {
        for (message, code, exit_code) in [
            (
                "SAVINGS_PLAN_EX_ANTE_COST_UNAVAILABLE: missing total",
                "savings_plan_ex_ante_cost_unavailable",
                30,
            ),
            (
                "SAVINGS_PLAN_CONFIRMATION_NOT_FOUND: missing",
                "savings_plan_confirmation_not_found",
                10,
            ),
            (
                "SAVINGS_PLAN_CONFIRMATION_EXPIRED: expired",
                "savings_plan_confirmation_expired",
                10,
            ),
            (
                "SAVINGS_PLAN_CONFIRMATION_ALREADY_USED: consumed",
                "savings_plan_confirmation_already_used",
                10,
            ),
            (
                "SAVINGS_PLAN_CONFIRMATION_CORRUPT: invalid JSON",
                "savings_plan_confirmation_corrupt",
                20,
            ),
            (
                "SAVINGS_PLAN_CONFIRMATION_ENV_MISMATCH: wrong env",
                "savings_plan_confirmation_env_mismatch",
                10,
            ),
            (
                "SAVINGS_PLAN_CONFIRMATION_ACCOUNT_MISMATCH: wrong account",
                "savings_plan_confirmation_account_mismatch",
                10,
            ),
            (
                "SAVINGS_PLAN_CONFIRMATION_PORTFOLIO_MISMATCH: wrong portfolio",
                "savings_plan_confirmation_portfolio_mismatch",
                10,
            ),
            (
                "SAVINGS_PLAN_CONFIRMATION_FIELDS_MISMATCH: costs changed",
                "savings_plan_confirmation_fields_mismatch",
                10,
            ),
            (
                "SAVINGS_PLAN_SUBMISSION_FAILED: backend rejected",
                "savings_plan_submission_failed",
                30,
            ),
            (
                "SAVINGS_PLAN_SUBMISSION_UNKNOWN: timeout",
                "savings_plan_submission_unknown",
                30,
            ),
        ] {
            let classified = classify_error(&anyhow!(message));
            assert_eq!(classified.code, code, "{message}");
            assert_eq!(classified.exit_code, exit_code, "{message}");
        }
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
        assert!(
            c.hints
                .iter()
                .any(|hint| hint.contains("check transactions or order status first"))
        );
    }

    #[test]
    fn classify_confirmation_appropriateness_warning_ack_required_error() {
        let err = anyhow!(
            "CONFIRMATION_APPROPRIATENESS_WARNING_ACK_REQUIRED: phase 1 requires acknowledgement of an appropriateness warning"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "confirmation_appropriateness_warning_ack_required");
        assert_eq!(c.exit_code, 10);
        assert!(
            c.hints
                .iter()
                .any(|hint| hint.contains("--acknowledge-appropriateness-warning"))
        );
    }

    #[test]
    fn classify_suitability_questionnaire_required_error() {
        let err = anyhow!(
            "SUITABILITY_QUESTIONNAIRE_REQUIRED: complete the required KNOCKOUT questionnaire outside the CLI before trading"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "suitability_questionnaire_required");
        assert_eq!(c.exit_code, 10);
    }

    #[test]
    fn classify_unsupported_trade_suitability_type_error() {
        let err = anyhow!(
            "UNSUPPORTED_TRADE_SUITABILITY_TYPE: ELTIF suitability is not supported in sc broker trade; use web or mobile for this product flow."
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "unsupported_trade_suitability_type");
        assert_eq!(c.exit_code, 10);
        assert!(c.hints.iter().any(|hint| hint.contains("web or mobile")));
    }

    #[test]
    fn classify_suitability_id_missing_error() {
        let err = anyhow!(
            "SUITABILITY_ID_MISSING: suitabilityId is missing for proceedable KNOCKOUT trade"
        );
        let c = classify_error(&err);
        assert_eq!(c.code, "suitability_id_missing");
        assert_eq!(c.exit_code, 30);
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
    fn concise_error_message_strips_local_read_only_prefix() {
        let text = format!(
            "{LOCAL_READ_ONLY_ERROR_PREFIX} local read-only mode blocks write operation 'BrokerAddToWatchlist'."
        );
        assert_eq!(
            concise_error_message(&text),
            "local read-only mode blocks write operation 'BrokerAddToWatchlist'."
        );
    }

    #[test]
    fn concise_error_message_strips_trade_control_prefix_and_reason_token() {
        let text = format!(
            "{LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX} local trade controls deny ISIN 'US0378331005' via denied_isins"
        );
        assert_eq!(
            concise_error_message(&text),
            "local trade controls deny ISIN 'US0378331005' via denied_isins"
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
