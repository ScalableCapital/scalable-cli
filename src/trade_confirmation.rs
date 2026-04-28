use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{config_dir_path, write_private_file_atomic};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfirmationFields {
    pub isin: String,
    pub amount: Option<String>,
    pub currency: String,
    pub venue: String,
    pub shares: String,
    pub entry_total: String,
    pub ongoing_total: String,
    pub exit_total: String,
    pub five_years_total: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ConfirmationPhase1Input {
    pub side: String,
    pub isin: String,
    pub amount: Option<String>,
    pub shares: Option<String>,
    pub venue: Option<String>,
    pub order_type: String,
    pub limit_price: Option<String>,
    pub stop_price: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeConfirmation {
    pub confirmation_id: String,
    #[serde(alias = "snapshot_checksum")]
    pub intent_checksum: String,
    pub nonce: String,
    pub created_at_epoch: i64,
    pub expires_at_epoch: i64,
    pub consumed_at_epoch: Option<i64>,
    pub env: String,
    pub account_id: String,
    pub portfolio_id: String,
    pub side: String,
    pub order_type: String,
    pub locale: String,
    pub venue_override: Option<String>,
    pub warning_version: Option<String>,
    #[serde(default)]
    pub requires_accept_unsuitable: bool,
    #[serde(default)]
    pub phase1_input: ConfirmationPhase1Input,
    pub fields: ConfirmationFields,
    pub snapshot_payload: Value,
    pub ex_ante_costs: Value,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConfirmationStore {
    pending: Option<TradeConfirmation>,
}

pub fn upsert_confirmation(mut confirmation: TradeConfirmation, _now_epoch: i64) -> Result<()> {
    confirmation.confirmation_id =
        required_non_empty(&confirmation.confirmation_id, "confirmation_id")?;
    confirmation.intent_checksum =
        required_non_empty(&confirmation.intent_checksum, "intent_checksum")?;
    confirmation.nonce = required_non_empty(&confirmation.nonce, "nonce")?;
    confirmation.env = required_non_empty(&confirmation.env, "env")?;
    confirmation.account_id = required_non_empty(&confirmation.account_id, "account_id")?;
    confirmation.portfolio_id = required_non_empty(&confirmation.portfolio_id, "portfolio_id")?;
    confirmation.side = required_non_empty(&confirmation.side, "side")?;
    confirmation.order_type = required_non_empty(&confirmation.order_type, "order_type")?;
    confirmation.locale = required_non_empty(&confirmation.locale, "locale")?;

    let store = ConfirmationStore {
        pending: Some(confirmation),
    };
    save_store(&store)
}

pub fn load_confirmation(confirmation_id: &str) -> Result<Option<TradeConfirmation>> {
    let id = confirmation_id.trim();
    if id.is_empty() {
        return Err(anyhow!("Confirmation id must not be empty"));
    }

    let store = load_store()?;
    Ok(match store.pending {
        Some(confirmation) if confirmation.confirmation_id == id => Some(confirmation),
        _ => None,
    })
}

pub fn mark_confirmation_consumed(confirmation_id: &str, now_epoch: i64) -> Result<()> {
    let id = confirmation_id.trim();
    if id.is_empty() {
        return Err(anyhow!("Confirmation id must not be empty"));
    }

    let mut store = load_store()?;
    let Some(confirmation) = store.pending.as_mut() else {
        return Err(anyhow!("Confirmation id '{}' not found", id));
    };
    if confirmation.confirmation_id != id {
        return Err(anyhow!("Confirmation id '{}' not found", id));
    }
    confirmation.consumed_at_epoch = Some(now_epoch);
    save_store(&store)
}

pub fn delete_confirmation_store() -> Result<()> {
    let path = confirmation_file_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| {
            format!(
                "Failed to delete trade confirmation store {}",
                path.display()
            )
        }),
    }
}

fn required_non_empty(input: &str, field: &str) -> Result<String> {
    let value = input.trim();
    if value.is_empty() {
        return Err(anyhow!(
            "Trade confirmation '{}' must be a non-empty string",
            field
        ));
    }
    Ok(value.to_string())
}

fn load_store() -> Result<ConfirmationStore> {
    let path = confirmation_file_path()?;
    if !path.exists() {
        return Ok(ConfirmationStore::default());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read trade confirmation store {}", path.display()))?;
    let store = serde_json::from_str::<ConfirmationStore>(&raw).with_context(|| {
        format!(
            "Invalid trade confirmation store JSON at {}",
            path.display()
        )
    })?;
    Ok(store)
}

fn save_store(store: &ConfirmationStore) -> Result<()> {
    let path = confirmation_file_path()?;
    let serialized = serde_json::to_string_pretty(store)?;
    write_private_file_atomic(&path, serialized.as_bytes()).with_context(|| {
        format!(
            "Failed to write trade confirmation store {}",
            path.display()
        )
    })?;
    Ok(())
}

fn confirmation_file_path() -> Result<PathBuf> {
    Ok(config_dir_path()?.join("trade_confirmation.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
                Some(v) => unsafe {
                    std::env::set_var(self.key, v);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    fn temp_config_dir() -> (TempDir, String) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_dir = tmp.path().to_string_lossy().to_string();
        (tmp, config_dir)
    }

    fn sample_confirmation(id: &str) -> TradeConfirmation {
        TradeConfirmation {
            confirmation_id: id.to_string(),
            intent_checksum: "sum1".to_string(),
            nonce: "nonce1".to_string(),
            created_at_epoch: 1_000,
            expires_at_epoch: 1_500,
            consumed_at_epoch: None,
            env: "dev".to_string(),
            account_id: "a1".to_string(),
            portfolio_id: "p1".to_string(),
            side: "buy".to_string(),
            order_type: "market".to_string(),
            locale: "en_DE".to_string(),
            venue_override: Some("MUNC".to_string()),
            warning_version: Some("v1".to_string()),
            requires_accept_unsuitable: false,
            phase1_input: ConfirmationPhase1Input {
                side: "buy".to_string(),
                isin: "US0378331005".to_string(),
                amount: Some("500".to_string()),
                shares: None,
                venue: Some("MUNC".to_string()),
                order_type: "market".to_string(),
                limit_price: None,
                stop_price: None,
            },
            fields: ConfirmationFields {
                isin: "US0378331005".to_string(),
                amount: Some("500".to_string()),
                currency: "EUR".to_string(),
                venue: "MUNC".to_string(),
                shares: "2".to_string(),
                entry_total: "1.2".to_string(),
                ongoing_total: "0.4".to_string(),
                exit_total: "0.3".to_string(),
                five_years_total: "5.0".to_string(),
            },
            snapshot_payload: serde_json::json!({"ok": true}),
            ex_ante_costs: serde_json::json!({"entryCosts": {"total": {"amount": "1.2"}}}),
        }
    }

    #[test]
    fn upsert_and_load_confirmation_round_trip() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        upsert_confirmation(sample_confirmation("scb1_1"), 1_000).expect("save");
        let loaded = load_confirmation("scb1_1")
            .expect("load")
            .expect("should exist");

        assert_eq!(loaded.confirmation_id, "scb1_1");
        assert_eq!(loaded.fields.currency, "EUR");
    }

    #[test]
    fn upsert_and_load_confirmation_round_trip_preserves_requires_accept_unsuitable() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        let mut confirmation = sample_confirmation("scb1_unsuitable");
        confirmation.requires_accept_unsuitable = true;

        upsert_confirmation(confirmation, 1_000).expect("save");
        let loaded = load_confirmation("scb1_unsuitable")
            .expect("load")
            .expect("should exist");

        assert!(loaded.requires_accept_unsuitable);
    }

    #[test]
    fn mark_confirmation_consumed_sets_timestamp() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        upsert_confirmation(sample_confirmation("scb1_2"), 1_000).expect("save");
        mark_confirmation_consumed("scb1_2", 1_111).expect("consume");
        let loaded = load_confirmation("scb1_2")
            .expect("load")
            .expect("should exist");

        assert_eq!(loaded.consumed_at_epoch, Some(1_111));
    }

    #[test]
    fn upsert_replaces_existing_pending_confirmation() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        upsert_confirmation(sample_confirmation("scb1_old"), 1_000).expect("save old");
        upsert_confirmation(sample_confirmation("scb1_new"), 1_001).expect("save new");

        let old_loaded = load_confirmation("scb1_old").expect("load old");
        let new_loaded = load_confirmation("scb1_new").expect("load new");

        assert!(
            old_loaded.is_none(),
            "old pending confirmation should be replaced"
        );
        assert!(
            new_loaded.is_some(),
            "new pending confirmation should remain"
        );
    }

    #[test]
    fn delete_confirmation_store_removes_existing_file() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        upsert_confirmation(sample_confirmation("scb1_delete"), 1_000).expect("save");

        let path = confirmation_file_path().expect("confirmation path");
        assert!(path.exists());

        delete_confirmation_store().expect("delete confirmation store");

        assert!(!path.exists());
    }

    #[test]
    fn delete_confirmation_store_ignores_missing_file() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        let path = confirmation_file_path().expect("confirmation path");
        assert!(!path.exists());

        delete_confirmation_store().expect("delete missing confirmation store");

        assert!(!path.exists());
    }

    #[test]
    fn load_confirmation_defaults_requires_accept_unsuitable_for_legacy_store() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        let path = confirmation_file_path().expect("confirmation path");
        fs::write(
            &path,
            r#"{
  "pending": {
    "confirmation_id": "scb1_legacy",
    "intent_checksum": "sum1",
    "nonce": "nonce1",
    "created_at_epoch": 1000,
    "expires_at_epoch": 1500,
    "consumed_at_epoch": null,
    "env": "dev",
    "account_id": "a1",
    "portfolio_id": "p1",
    "side": "buy",
    "order_type": "market",
    "locale": "en_DE",
    "venue_override": "MUNC",
    "warning_version": "v1",
    "phase1_input": {
      "side": "buy",
      "isin": "US0378331005",
      "amount": "500",
      "shares": null,
      "venue": "MUNC",
      "order_type": "market",
      "limit_price": null,
      "stop_price": null
    },
    "fields": {
      "isin": "US0378331005",
      "amount": "500",
      "currency": "EUR",
      "venue": "MUNC",
      "shares": "2",
      "entry_total": "1.2",
      "ongoing_total": "0.4",
      "exit_total": "0.3",
      "five_years_total": "5.0"
    },
    "snapshot_payload": {"ok": true},
    "ex_ante_costs": {"entryCosts": {"total": {"amount": "1.2"}}}
  }
}"#,
        )
        .expect("write legacy store");

        let loaded = load_confirmation("scb1_legacy")
            .expect("load")
            .expect("should exist");

        assert!(!loaded.requires_accept_unsuitable);
    }
}
