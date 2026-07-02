use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::overnight_queries::OvernightSummaryInput;
use crate::overnight_shared::{DiscoveredOvernightAccount, OvernightOwnerKind};

const OVERNIGHT_FALLBACK_NAME: &str = "Overnight";
const ACTIVE_STATE: &str = "ACTIVE";
const COMPLETE_ONBOARDING_STATE: &str = "COMPLETE";

pub(crate) fn project_overnight_discovery_response(
    response: &Value,
) -> Result<Vec<DiscoveredOvernightAccount>> {
    let savings_accounts = optional_array(
        response
            .get("account")
            .and_then(|value| value.get("savingsAccounts")),
    );
    let minors = optional_array(
        response
            .get("productList")
            .and_then(|value| value.get("minors")),
    );

    let mut discovered = Vec::new();

    for savings_account in savings_accounts {
        let Some(typename) = optional_non_empty_string(savings_account.get("__typename")) else {
            continue;
        };
        if typename != "OvernightSavingsAccount" {
            continue;
        }

        let Some(savings_account_id) = optional_non_empty_string(savings_account.get("id")) else {
            continue;
        };
        let Some(state) = optional_non_empty_string(savings_account.get("state")) else {
            continue;
        };
        let display_name = optional_non_empty_string(
            savings_account
                .get("personalizations")
                .and_then(|value| value.get("name")),
        )
        .unwrap_or_else(|| OVERNIGHT_FALLBACK_NAME.to_string());

        discovered.push(DiscoveredOvernightAccount {
            savings_account_id,
            display_name,
            owner_kind: OvernightOwnerKind::Personal,
            is_active: state == ACTIVE_STATE,
        });
    }

    for minor in minors {
        let Some(product_type) = optional_non_empty_string(minor.get("type")) else {
            continue;
        };
        if !is_overnight_minor_type(&product_type) {
            continue;
        }

        let Some(savings_account_id) = optional_non_empty_string(minor.get("id")) else {
            continue;
        };
        let Some(onboarding_state) = optional_non_empty_string(minor.get("onboardingState")) else {
            continue;
        };
        let display_name = build_minor_display_name(minor);
        let state = optional_non_empty_string(minor.get("state"));
        let is_active = onboarding_state == COMPLETE_ONBOARDING_STATE
            && state.as_deref() != Some("CANCELLED")
            && state.as_deref() != Some("CLOSED");

        discovered.push(DiscoveredOvernightAccount {
            savings_account_id,
            display_name,
            owner_kind: OvernightOwnerKind::Junior,
            is_active,
        });
    }

    discovered.sort_by(|left, right| {
        left.savings_account_id
            .cmp(&right.savings_account_id)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    discovered.dedup_by(|left, right| left.savings_account_id == right.savings_account_id);

    Ok(discovered)
}

pub(crate) fn project_overnight_summary_response(
    input: &OvernightSummaryInput,
    response: &Value,
) -> Result<Value> {
    let savings_account = required_value(
        response
            .get("account")
            .and_then(|value| value.get("savingsAccount")),
        "account.savingsAccount",
    )?;
    let savings_account_id =
        required_non_empty_string(savings_account.get("id"), "account.savingsAccount.id")?;
    if savings_account_id != input.savings_account_id() {
        return Err(anyhow!(
            "Overnight response invalid: account.savingsAccount.id did not match the requested savings account id"
        ));
    }

    let interests = required_value(
        savings_account.get("interests"),
        "account.savingsAccount.interests",
    )?;
    let next_payout_date = match savings_account
        .get("nextPayoutDate")
        .or_else(|| interests.get("nextPayoutDate"))
    {
        Some(value) => {
            epoch_seconds_value_or_null(value, "account.savingsAccount.nextPayoutDate.epochSecond")?
        }
        None => Value::Null,
    };

    Ok(json!({
        "interest_rate": required_value(
            interests.get("depositInterestRate"),
            "account.savingsAccount.interests.depositInterestRate",
        )?,
        "balance": required_value(
            savings_account.get("totalAmount"),
            "account.savingsAccount.totalAmount",
        )?,
        "current_accrued_amount": required_value(
            interests.get("currentAccruedAmount"),
            "account.savingsAccount.interests.currentAccruedAmount",
        )?,
        "current_interest_bearing_amount": required_value(
            interests.get("currentInterestBearingAmount"),
            "account.savingsAccount.interests.currentInterestBearingAmount",
        )?,
        "deposit_accrued_lifetime_amount": required_value(
            interests.get("depositAccruedLifetimeAmount"),
            "account.savingsAccount.interests.depositAccruedLifetimeAmount",
        )?,
        "estimated_next_payout_amount": required_value(
            interests.get("estimatedNextPayoutAmount"),
            "account.savingsAccount.interests.estimatedNextPayoutAmount",
        )?,
        "next_payout_date": next_payout_date,
    }))
}

fn build_minor_display_name(minor: &Value) -> String {
    optional_non_empty_string(minor.get("name"))
        .or_else(|| {
            minor
                .get("owners")
                .and_then(Value::as_array)
                .and_then(|owners| owners.first())
                .and_then(|owner| optional_non_empty_string(owner.get("firstName")))
        })
        .or_else(|| {
            minor.get("productOffering").and_then(|offering| {
                optional_non_empty_string(offering.get("displayName"))
                    .or_else(|| optional_non_empty_string(offering.get("name")))
            })
        })
        .unwrap_or_else(|| OVERNIGHT_FALLBACK_NAME.to_string())
}

fn is_overnight_minor_type(product_type: &str) -> bool {
    matches!(
        product_type,
        "OVERNIGHT_SAVINGS" | "OvernightSavings" | "overnightSavings"
    )
}

fn optional_array(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn required_value<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a Value> {
    value.ok_or_else(|| anyhow!("Overnight response invalid: missing {path}"))
}

fn required_non_empty_string(value: Option<&Value>, path: &str) -> Result<String> {
    let value = value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Overnight response invalid: missing {path}"))?;
    Ok(value.to_string())
}

fn optional_non_empty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn epoch_seconds_value_or_null(value: &Value, path: &str) -> Result<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }

    match value.get("epochSecond") {
        Some(Value::Null) => Ok(Value::Null),
        Some(epoch_second) => {
            let epoch_second = epoch_second.as_i64().ok_or_else(|| {
                anyhow!("Overnight response invalid: invalid {path} (expected integer epochSecond)")
            })?;
            let timestamp = DateTime::<Utc>::from_timestamp(epoch_second, 0).ok_or_else(|| {
                anyhow!("Overnight response invalid: invalid {path} (unix epoch out of range)")
            })?;
            Ok(Value::String(timestamp.to_rfc3339()))
        }
        None => Err(anyhow!("Overnight response invalid: missing {path}")),
    }
}

#[cfg(test)]
#[path = "overnight_projections_tests.rs"]
mod tests;
