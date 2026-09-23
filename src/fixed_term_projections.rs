use anyhow::{Result, anyhow, bail};
use chrono::DateTime;
use serde_json::{Value, json};

pub(crate) const NOT_FOUND: &str =
    "Fixed-Term account not found: the requested account is not available";

pub(crate) fn project_list(response: &Value) -> Result<Value> {
    let account = required(response, "account")?;
    if account.is_null() {
        return Err(invalid("account"));
    }
    let accounts = required(account, "fixedTermSavingsAccounts")?
        .as_array()
        .ok_or_else(|| invalid("fixedTermSavingsAccounts"))?;
    let mut projected = Vec::new();
    for account in accounts {
        if is_supported(account)? {
            projected.push(summary(account)?);
        }
    }
    projected.sort_by(|left, right| {
        left["fixed_term_account_id"]
            .as_str()
            .cmp(&right["fixed_term_account_id"].as_str())
    });
    Ok(json!({"accounts": projected}))
}

pub(crate) fn project_details(response: &Value, requested_id: &str) -> Result<Value> {
    let account = required(response, "account")?;
    if account.is_null() {
        bail!(NOT_FOUND);
    }
    let account = required(account, "fixedTermSavingsAccount")?;
    if account.is_null() {
        bail!(NOT_FOUND);
    }
    if text(account, "id")? != requested_id {
        bail!("Fixed-Term response invalid: account id did not match the requested account");
    }
    if !is_supported(account)? {
        bail!(NOT_FOUND);
    }
    let mut output = summary(account)?;
    output["contractual_start_date"] = date(account, "startDate")?;
    for (source, target) in [
        ("effectiveInterestRate", "effective_interest_rate"),
        ("currentAccruedAmount", "current_accrued_gross_interest"),
        ("estimatedPayoutAmount", "estimated_payout_amount"),
    ] {
        output[target] = decimal(account, source, true)?;
    }
    output["auto_renewal_enabled"] = json!(
        required(account, "autoRenewal")?
            .as_bool()
            .ok_or_else(|| invalid("autoRenewal"))?
    );
    let allocation = account
        .get("trusteeBalanceBreakdown")
        .unwrap_or(&Value::Null);
    output["allocation"] = if allocation.is_null() {
        Value::Null
    } else {
        let balances = required(allocation, "balances")?
            .as_array()
            .ok_or_else(|| invalid("trusteeBalanceBreakdown.balances"))?
            .iter()
            .map(|balance| {
                Ok(json!({
                    "trustee_name": text(balance, "trusteeName")?,
                    "amount": decimal(balance, "balance", false)?,
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        json!({
            "balances": balances,
            "snapshot_date_time": timestamp(allocation.get("snapshotDateTime").unwrap_or(&Value::Null))?,
        })
    };
    Ok(output)
}

fn is_supported(account: &Value) -> Result<bool> {
    let is_personal = match text(account, "configuration")? {
        "SINGLE" => true,
        "JUNIOR" => false,
        _ => return Err(invalid("configuration")),
    };
    let is_supported_state = match text(account, "state")? {
        "PENDING" | "ACTIVE" | "CANCELLATION_REQUESTED" => true,
        "CREATED" | "DOI_ACCEPTED" => false,
        _ => return Err(invalid("state")),
    };
    Ok(is_personal && is_supported_state)
}

fn summary(account: &Value) -> Result<Value> {
    let term = account.get("fixedTerm").unwrap_or(&Value::Null);
    if !term.is_null() && term.as_i64().is_none_or(|months| months <= 0) {
        return Err(invalid("fixedTerm"));
    }
    let name = account
        .get("personalizations")
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Fixed Term");
    Ok(json!({
        "fixed_term_account_id": text(account, "id")?,
        "display_name": name,
        "owner_kind": "personal",
        "state": text(account, "state")?,
        "deposit_amount": decimal(account, "totalAmount", false)?,
        "currency": "EUR",
        "fixed_term_months": term,
        "nominal_interest_rate": decimal(account, "interestRate", false)?,
        "maturity_date": date(account, "endDate")?,
    }))
}

fn required<'a>(value: &'a Value, field: &str) -> Result<&'a Value> {
    value.get(field).ok_or_else(|| invalid(field))
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    required(value, field)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid(field))
}

fn invalid(field: &str) -> anyhow::Error {
    anyhow!("Fixed-Term response invalid: invalid or missing {field}")
}

fn decimal(value: &Value, field: &str, nullable: bool) -> Result<Value> {
    let raw = value.get(field).unwrap_or(&Value::Null);
    if nullable && raw.is_null() {
        return Ok(Value::Null);
    }
    let text = match raw {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return Err(invalid(field)),
    };
    // Validate syntax without parsing through floating point or changing precision.
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(&text);
    let mut parts = unsigned.split(['e', 'E']);
    let mantissa = parts.next().unwrap_or_default();
    let exponent = parts.next();
    let valid_mantissa = mantissa.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && mantissa.bytes().any(|b| b.is_ascii_digit())
        && mantissa.bytes().filter(|b| *b == b'.').count() <= 1;
    let valid_exponent = exponent.is_none_or(|e| {
        let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    });
    if !valid_mantissa || !valid_exponent || parts.next().is_some() {
        return Err(invalid(field));
    }
    Ok(json!(text))
}

fn date(value: &Value, field: &str) -> Result<Value> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(date) => timestamp(required(date, "time")?),
    }
}

fn timestamp(value: &Value) -> Result<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let raw = value.as_str().ok_or_else(|| invalid("timestamp"))?;
    DateTime::parse_from_rfc3339(raw).map_err(|_| invalid("timestamp"))?;
    Ok(value.clone())
}
