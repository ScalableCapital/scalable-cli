use anyhow::Result;
use serde_json::Value;

use crate::config::AppConfig;
use crate::overnight_query_execution::execute_overnight_summary;
use crate::session::SessionManager;

pub(crate) enum HumanOvernightOutput {
    Json(Value, bool),
    Text(Vec<String>),
}

pub(crate) fn run_overnight_command_human(
    args: crate::cli::OvernightArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<HumanOvernightOutput> {
    let compact = args.json;
    let payload = execute_overnight_summary(args, config, session_manager)?;
    if compact {
        return Ok(HumanOvernightOutput::Json(payload, true));
    }
    Ok(HumanOvernightOutput::Text(render_overnight_summary_text(
        &payload,
    )))
}

pub(crate) fn run_overnight_command_machine(
    args: crate::cli::OvernightArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    execute_overnight_summary(args, config, session_manager)
}

fn render_overnight_summary_text(payload: &Value) -> Vec<String> {
    let account = payload.get("account").unwrap_or(&Value::Null);
    let result = payload.get("result").unwrap_or(&Value::Null);

    vec![
        format!(
            "savings_account_id: {}",
            display_value(payload.get("savings_account_id"))
        ),
        format!(
            "account_name: {}",
            display_value(account.get("display_name"))
        ),
        format!("owner_kind: {}", display_value(account.get("owner_kind"))),
        format!(
            "interest_rate: {}",
            display_value(result.get("interest_rate"))
        ),
        format!("balance: {}", display_value(result.get("balance"))),
        format!(
            "current_interest_bearing_amount: {}",
            display_value(result.get("current_interest_bearing_amount"))
        ),
        format!(
            "current_accrued_amount: {}",
            display_value(result.get("current_accrued_amount"))
        ),
        format!(
            "estimated_next_payout_amount: {}",
            display_value(result.get("estimated_next_payout_amount"))
        ),
        format!(
            "next_payout_date: {}",
            display_value(result.get("next_payout_date"))
        ),
        format!(
            "deposit_accrued_lifetime_amount: {}",
            display_value(result.get("deposit_accrued_lifetime_amount"))
        ),
    ]
}

fn display_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::Null) | None => "<none>".to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_overnight_summary_text_uses_single_interest_rate_surface() {
        let lines = render_overnight_summary_text(&json!({
            "savings_account_id": "sav-1",
            "account": {
                "display_name": "Tagesgeld",
                "owner_kind": "personal",
            },
            "result": {
                "interest_rate": "0.02",
                "balance": "1001.23",
                "current_interest_bearing_amount": "1000",
                "current_accrued_amount": "1.23",
                "estimated_next_payout_amount": "0.98",
                "next_payout_date": "1970-01-01T00:00:00+00:00",
                "deposit_accrued_lifetime_amount": "12.34",
            }
        }));

        assert!(lines.contains(&"interest_rate: 0.02".to_string()));
        assert!(!lines.iter().any(|line| line.contains("interest_tier")));
        assert!(!lines.iter().any(|line| line.contains("effective_yearly")));
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("deposit_interest_rate"))
        );
    }
}
