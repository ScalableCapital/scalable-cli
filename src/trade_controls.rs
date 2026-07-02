use std::cmp::Ordering;
use std::collections::BTreeSet;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::config::{AppConfig, TradeControlsConfig};

pub(crate) const LOCAL_TRADE_CONTROL_ISIN_NOT_ALLOWED_PREFIX: &str =
    "LOCAL_TRADE_CONTROL:isin_not_allowed:";
pub(crate) const LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX: &str = "LOCAL_TRADE_CONTROL:isin_denied:";
pub(crate) const LOCAL_TRADE_CONTROL_ORDER_NOTIONAL_EXCEEDED_PREFIX: &str =
    "LOCAL_TRADE_CONTROL:order_notional_exceeded:";
const ESTIMATED_ORDER_VOLUME_SCALE: usize = 4;

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TradeControlsPolicy {
    allowed_isins: Option<BTreeSet<String>>,
    denied_isins: BTreeSet<String>,
    allowed_isins_configured: bool,
    denied_isins_configured: bool,
    max_order_notional_str: Option<String>,
}

impl TradeControlsPolicy {
    pub(crate) fn from_app_config(config: &AppConfig) -> Self {
        match &config.trade_controls {
            Some(controls) => Self::from_config(controls),
            None => Self::default(),
        }
    }

    pub(crate) fn from_config(config: &TradeControlsConfig) -> Self {
        let allowed_isins = config
            .allowed_isins
            .as_ref()
            .map(|items| items.iter().map(|value| canonicalize_isin(value)).collect());
        let denied_isins = config
            .denied_isins
            .as_ref()
            .map(|items| items.iter().map(|value| canonicalize_isin(value)).collect())
            .unwrap_or_default();

        let max_order_notional_str = config
            .max_order_notional
            .as_ref()
            .map(|value| normalize_decimal_str(value));

        Self {
            allowed_isins,
            denied_isins,
            allowed_isins_configured: config.allowed_isins.is_some(),
            denied_isins_configured: config.denied_isins.is_some(),
            max_order_notional_str,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.isin_controls_active() || self.max_order_notional_active()
    }

    pub(crate) fn isin_controls_active(&self) -> bool {
        self.allowed_isins_configured || self.denied_isins_configured
    }

    pub(crate) fn max_order_notional_active(&self) -> bool {
        self.max_order_notional_str.is_some()
    }

    pub(crate) fn check_isin(&self, isin: &str) -> Result<()> {
        if !self.isin_controls_active() {
            return Ok(());
        }

        let normalized = canonicalize_isin(isin);
        if self.denied_isins.contains(&normalized) {
            bail!(
                "{LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX} local trade controls deny ISIN '{normalized}' via denied_isins"
            );
        }

        if let Some(allowed) = &self.allowed_isins
            && !allowed.contains(&normalized)
        {
            bail!(
                "{LOCAL_TRADE_CONTROL_ISIN_NOT_ALLOWED_PREFIX} local trade controls require ISIN '{normalized}' to be present in allowed_isins"
            );
        }

        Ok(())
    }

    pub(crate) fn check_estimated_order_volume(&self, estimated_order_volume: f64) -> Result<()> {
        let Some(configured) = self.max_order_notional_str.as_deref() else {
            return Ok(());
        };

        // `prepare_trade(...)` already rounds `estimated_order_volume` to the 4dp ex-ante
        // contract before policy enforcement, so this check intentionally compares against the
        // prepared value rather than any higher-precision pre-rounded intermediate.
        let actual =
            canonical_decimal_from_f64(estimated_order_volume, ESTIMATED_ORDER_VOLUME_SCALE);
        if compare_positive_decimal_strs(&actual, configured) == Ordering::Greater {
            bail!(
                "{LOCAL_TRADE_CONTROL_ORDER_NOTIONAL_EXCEEDED_PREFIX} local trade controls block estimated order notional '{actual}' because it exceeds max_order_notional '{configured}'"
            );
        }

        Ok(())
    }

    pub(crate) fn capabilities_payload(&self) -> Value {
        json!({
            "enabled": self.enabled(),
            "isin_controls_active": self.isin_controls_active(),
            "allowed_isins_configured": self.allowed_isins_configured,
            "denied_isins_configured": self.denied_isins_configured,
            "max_order_notional_active": self.max_order_notional_active(),
            "enforced_on": [
                "broker.trade.buy.phase1",
                "broker.trade.buy.phase2",
                "broker.trade.sell.phase1",
                "broker.trade.sell.phase2"
            ],
            "allowed_isins": self.allowed_isins_list(),
            "denied_isins": self.denied_isins_list(),
            "isin_resolution": "allow_minus_deny",
            "max_order_notional": self.max_order_notional_str,
        })
    }

    fn allowed_isins_list(&self) -> Vec<String> {
        self.allowed_isins
            .as_ref()
            .map(|items| items.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn denied_isins_list(&self) -> Vec<String> {
        self.denied_isins.iter().cloned().collect()
    }
}

fn canonicalize_isin(value: &str) -> String {
    value.trim().to_uppercase()
}

fn canonical_decimal_from_f64(value: f64, scale: usize) -> String {
    normalize_decimal_str(&format!("{value:.scale$}"))
}

fn compare_positive_decimal_strs(left: &str, right: &str) -> Ordering {
    let (left_integer, left_fraction) = split_decimal_str(left);
    let (right_integer, right_fraction) = split_decimal_str(right);

    left_integer
        .len()
        .cmp(&right_integer.len())
        .then_with(|| left_integer.cmp(right_integer))
        .then_with(|| compare_fractional_parts(left_fraction, right_fraction))
}

fn split_decimal_str(value: &str) -> (&str, &str) {
    value.split_once('.').unwrap_or((value, ""))
}

fn compare_fractional_parts(left: &str, right: &str) -> Ordering {
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_digit = left.as_bytes().get(index).copied().unwrap_or(b'0');
        let right_digit = right.as_bytes().get(index).copied().unwrap_or(b'0');
        let ordering = left_digit.cmp(&right_digit);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    Ordering::Equal
}

fn normalize_decimal_str(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "0".to_string();
    }

    let mut chars = trimmed.chars();
    let negative = matches!(chars.next(), Some('-'));
    let unsigned = if negative || trimmed.starts_with('+') {
        &trimmed[1..]
    } else {
        trimmed
    };

    if unsigned.is_empty() {
        return "0".to_string();
    }

    let parts = unsigned.split('.').collect::<Vec<_>>();
    if parts.len() > 2
        || !parts
            .iter()
            .all(|segment| segment.chars().all(|ch| ch.is_ascii_digit()))
    {
        return trimmed.to_string();
    }

    let integer_raw = parts[0].trim_start_matches('0');
    let integer = if integer_raw.is_empty() {
        "0"
    } else {
        integer_raw
    };
    let fraction = if parts.len() == 2 {
        parts[1].trim_end_matches('0')
    } else {
        ""
    };

    let mut normalized = if fraction.is_empty() {
        integer.to_string()
    } else {
        format!("{integer}.{fraction}")
    };

    if negative && normalized != "0" {
        normalized = format!("-{normalized}");
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RuntimeAuthConfig, SessionBackendPreference};
    use crate::trade::round_estimated_order_volume_for_ex_ante;

    fn sample_config_with_controls(controls: Option<TradeControlsConfig>) -> AppConfig {
        AppConfig {
            auth: RuntimeAuthConfig {
                session_backend: SessionBackendPreference::File,
                ..RuntimeAuthConfig::default()
            },
            trade_controls: controls,
        }
    }

    #[test]
    fn policy_is_disabled_without_trade_controls() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(None));
        assert!(!policy.enabled());
        assert!(!policy.isin_controls_active());
        assert!(!policy.max_order_notional_active());
    }

    #[test]
    fn policy_deduplicates_and_canonicalizes_isins() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: Some(vec![
                    " us0378331005 ".to_string(),
                    "US0378331005".to_string(),
                    "ie00b4l5y983".to_string(),
                ]),
                denied_isins: Some(vec![
                    " us88160r1014 ".to_string(),
                    "US88160R1014".to_string(),
                ]),
                max_order_notional: Some("001000.00".to_string()),
            },
        )));

        assert_eq!(
            policy.allowed_isins_list(),
            vec!["IE00B4L5Y983".to_string(), "US0378331005".to_string()]
        );
        assert_eq!(policy.denied_isins_list(), vec!["US88160R1014".to_string()]);
        assert_eq!(policy.max_order_notional_str.as_deref(), Some("1000"));
    }

    #[test]
    fn denylist_wins_over_allowlist() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: Some(vec!["US0378331005".to_string()]),
                denied_isins: Some(vec!["US0378331005".to_string()]),
                max_order_notional: None,
            },
        )));

        let err = policy
            .check_isin("US0378331005")
            .expect_err("denylist should win");
        assert!(
            err.to_string()
                .contains(LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX)
        );
    }

    #[test]
    fn denylist_precedence_applies_even_when_allowlist_would_reject() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: Some(vec!["IE00B4L5Y983".to_string()]),
                denied_isins: Some(vec!["US0378331005".to_string()]),
                max_order_notional: None,
            },
        )));

        let err = policy
            .check_isin("US0378331005")
            .expect_err("denylist should remain the more specific rejection");
        assert!(
            err.to_string()
                .contains(LOCAL_TRADE_CONTROL_ISIN_DENIED_PREFIX)
        );
    }

    #[test]
    fn empty_allowlist_blocks_all_isins() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: Some(vec![]),
                denied_isins: None,
                max_order_notional: None,
            },
        )));

        let err = policy
            .check_isin("US0378331005")
            .expect_err("empty allowlist should deny all");
        assert!(
            err.to_string()
                .contains(LOCAL_TRADE_CONTROL_ISIN_NOT_ALLOWED_PREFIX)
        );
    }

    #[test]
    fn deny_only_mode_allows_non_denied_isins() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: None,
                denied_isins: Some(vec!["US88160R1014".to_string()]),
                max_order_notional: None,
            },
        )));

        policy
            .check_isin("US0378331005")
            .expect("non-denied ISIN should pass");
    }

    #[test]
    fn exact_notional_limit_is_allowed() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: None,
                denied_isins: None,
                max_order_notional: Some("454.1049".to_string()),
            },
        )));

        policy
            .check_estimated_order_volume(454.1049)
            .expect("exact equality should pass");
    }

    #[test]
    fn exceeding_notional_limit_returns_stable_prefix() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: None,
                denied_isins: None,
                max_order_notional: Some("1000".to_string()),
            },
        )));

        let err = policy
            .check_estimated_order_volume(1000.0001)
            .expect_err("above limit should fail");
        assert!(
            err.to_string()
                .contains(LOCAL_TRADE_CONTROL_ORDER_NOTIONAL_EXCEEDED_PREFIX)
        );
    }

    #[test]
    fn decimal_comparison_avoids_binary_floating_drift() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: None,
                denied_isins: None,
                max_order_notional: Some("0.3".to_string()),
            },
        )));

        policy
            .check_estimated_order_volume(0.1 + 0.2)
            .expect("0.1 + 0.2 should compare equal to configured 0.3 after 4dp normalization");
    }

    #[test]
    fn decimal_comparison_preserves_exact_config_precision() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: None,
                denied_isins: None,
                max_order_notional: Some("1000.00001".to_string()),
            },
        )));

        policy
            .check_estimated_order_volume(1000.0)
            .expect("prepared order volume below exact decimal limit should pass");

        let err = policy
            .check_estimated_order_volume(1000.0001)
            .expect_err("prepared order volume above exact decimal limit should fail");
        assert!(
            err.to_string()
                .contains(LOCAL_TRADE_CONTROL_ORDER_NOTIONAL_EXCEEDED_PREFIX)
        );
    }

    #[test]
    fn notional_control_enforces_prepared_rounded_trade_volume() {
        let policy = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: None,
                denied_isins: None,
                max_order_notional: Some("1000.00001".to_string()),
            },
        )));

        let raw_estimated_order_volume = 1000.00004;
        let prepared_estimated_order_volume =
            round_estimated_order_volume_for_ex_ante(raw_estimated_order_volume);

        assert_eq!(prepared_estimated_order_volume, 1000.0);
        policy
            .check_estimated_order_volume(prepared_estimated_order_volume)
            .expect("trade controls should enforce the same 4dp prepared value produced by the trade flow");
    }

    #[test]
    fn capabilities_payload_distinguishes_inactive_and_empty_allowlist() {
        let disabled = TradeControlsPolicy::from_app_config(&sample_config_with_controls(None))
            .capabilities_payload();
        assert_eq!(disabled["enabled"], Value::Bool(false));
        assert_eq!(disabled["allowed_isins_configured"], Value::Bool(false));

        let deny_all = TradeControlsPolicy::from_app_config(&sample_config_with_controls(Some(
            TradeControlsConfig {
                allowed_isins: Some(vec![]),
                denied_isins: None,
                max_order_notional: None,
            },
        )))
        .capabilities_payload();
        assert_eq!(deny_all["enabled"], Value::Bool(true));
        assert_eq!(deny_all["isin_controls_active"], Value::Bool(true));
        assert_eq!(deny_all["allowed_isins_configured"], Value::Bool(true));
        assert_eq!(deny_all["allowed_isins"], json!([]));
    }
}
