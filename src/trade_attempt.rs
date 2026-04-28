use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::config::{TargetEnv, config_dir_path, write_private_file_atomic};

const ATTEMPT_REUSE_WINDOW_SECONDS: i64 = 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TradeAttemptStatus {
    Prepared,
    SubmitInFlight,
    Submitted,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TradeAttempt {
    pub idempotency_key: String,
    pub intent_hash: String,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
    pub status: TradeAttemptStatus,
    pub order_id: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTradeAttempt {
    pub idempotency_key: String,
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedTradeAttempt {
    pub idempotency_key: String,
    pub order_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TradeAttemptStore {
    dev: Option<TradeAttempt>,
    prod: Option<TradeAttempt>,
}

pub fn start_or_reuse_attempt(
    env: TargetEnv,
    intent_hash: &str,
    now_epoch: i64,
) -> Result<ResolvedTradeAttempt> {
    let intent_hash = intent_hash.trim();
    if intent_hash.is_empty() {
        return Err(anyhow!("Trade attempt intent hash must not be empty"));
    }

    let mut store = load_store()?;
    let mut reused_key = None::<String>;
    {
        let existing = selected_mut(&mut store, env);
        if let Some(attempt) = existing {
            let fresh = now_epoch - attempt.updated_at_epoch <= ATTEMPT_REUSE_WINDOW_SECONDS;
            let reusable = fresh
                && attempt.intent_hash == intent_hash
                && attempt.status != TradeAttemptStatus::Submitted;
            if reusable {
                attempt.updated_at_epoch = now_epoch;
                reused_key = Some(attempt.idempotency_key.clone());
            }
        }
    }

    if let Some(idempotency_key) = reused_key {
        save_store(&store)?;
        return Ok(ResolvedTradeAttempt {
            idempotency_key,
            reused: true,
        });
    }

    let idempotency_key = generate_idempotency_key(now_epoch);
    let next = TradeAttempt {
        idempotency_key: idempotency_key.clone(),
        intent_hash: intent_hash.to_string(),
        created_at_epoch: now_epoch,
        updated_at_epoch: now_epoch,
        status: TradeAttemptStatus::Prepared,
        order_id: None,
        last_error: None,
    };
    *selected_mut(&mut store, env) = Some(next);
    save_store(&store)?;

    Ok(ResolvedTradeAttempt {
        idempotency_key,
        reused: false,
    })
}

pub fn mark_submit_in_flight(env: TargetEnv, idempotency_key: &str, now_epoch: i64) -> Result<()> {
    update_attempt(env, idempotency_key, now_epoch, |attempt| {
        attempt.status = TradeAttemptStatus::SubmitInFlight;
        attempt.last_error = None;
    })
}

pub fn mark_submitted(
    env: TargetEnv,
    idempotency_key: &str,
    order_id: &str,
    now_epoch: i64,
) -> Result<()> {
    let order_id = order_id.trim();
    if order_id.is_empty() {
        return Err(anyhow!("Order id must not be empty when marking submitted"));
    }

    update_attempt(env, idempotency_key, now_epoch, |attempt| {
        attempt.status = TradeAttemptStatus::Submitted;
        attempt.order_id = Some(order_id.to_string());
        attempt.last_error = None;
    })
}

pub fn mark_failed(
    env: TargetEnv,
    idempotency_key: &str,
    error: &str,
    now_epoch: i64,
) -> Result<()> {
    let message = error.trim();
    if message.is_empty() {
        return Err(anyhow!(
            "Failure message must not be empty when marking attempt failed"
        ));
    }

    update_attempt(env, idempotency_key, now_epoch, |attempt| {
        attempt.status = TradeAttemptStatus::Failed;
        attempt.last_error = Some(message.to_string());
    })
}

pub fn load_attempt(env: TargetEnv) -> Result<Option<TradeAttempt>> {
    let store = load_store()?;
    Ok(match env {
        TargetEnv::Dev => store.dev,
        TargetEnv::Prod => store.prod,
    })
}

pub fn load_recent_submitted_attempt(
    env: TargetEnv,
    intent_hash: &str,
    now_epoch: i64,
) -> Result<Option<SubmittedTradeAttempt>> {
    let normalized_hash = intent_hash.trim();
    if normalized_hash.is_empty() {
        return Err(anyhow!("Trade attempt intent hash must not be empty"));
    }

    let Some(attempt) = load_attempt(env)? else {
        return Ok(None);
    };

    let fresh = now_epoch - attempt.updated_at_epoch <= ATTEMPT_REUSE_WINDOW_SECONDS;
    if !fresh
        || attempt.intent_hash != normalized_hash
        || attempt.status != TradeAttemptStatus::Submitted
    {
        return Ok(None);
    }

    let Some(order_id) = attempt
        .order_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    Ok(Some(SubmittedTradeAttempt {
        idempotency_key: attempt.idempotency_key,
        order_id: order_id.to_string(),
    }))
}

pub fn delete_attempt_store() -> Result<()> {
    let path = attempt_file_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("Failed to delete trade attempt store {}", path.display())),
    }
}

fn update_attempt<F>(
    env: TargetEnv,
    idempotency_key: &str,
    now_epoch: i64,
    mut update: F,
) -> Result<()>
where
    F: FnMut(&mut TradeAttempt),
{
    let key = idempotency_key.trim();
    if key.is_empty() {
        return Err(anyhow!("Idempotency key must not be empty"));
    }

    let mut store = load_store()?;
    let attempt = selected_mut(&mut store, env)
        .as_mut()
        .ok_or_else(|| anyhow!("No stored trade attempt for environment {}", env.as_str()))?;
    if attempt.idempotency_key != key {
        return Err(anyhow!(
            "Stored trade attempt idempotency key does not match requested key"
        ));
    }
    attempt.updated_at_epoch = now_epoch;
    update(attempt);
    save_store(&store)?;
    Ok(())
}

fn generate_idempotency_key(_now_epoch: i64) -> String {
    let mut bytes = rand::random::<[u8; 16]>();
    // RFC 4122 variant + version 4 shape for backend UUID parsing.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn selected_mut(store: &mut TradeAttemptStore, env: TargetEnv) -> &mut Option<TradeAttempt> {
    match env {
        TargetEnv::Dev => &mut store.dev,
        TargetEnv::Prod => &mut store.prod,
    }
}

fn load_store() -> Result<TradeAttemptStore> {
    let path = attempt_file_path()?;
    if !path.exists() {
        return Ok(TradeAttemptStore::default());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read trade attempt store {}", path.display()))?;
    let store = serde_json::from_str::<TradeAttemptStore>(&raw)
        .with_context(|| format!("Invalid trade attempt store JSON {}", path.display()))?;
    Ok(store)
}

fn save_store(store: &TradeAttemptStore) -> Result<()> {
    let path = attempt_file_path()?;
    let serialized = serde_json::to_string_pretty(store)?;
    write_private_file_atomic(&path, serialized.as_bytes())
        .with_context(|| format!("Failed to write trade attempt store {}", path.display()))?;
    Ok(())
}

fn attempt_file_path() -> Result<PathBuf> {
    Ok(config_dir_path()?.join("trade_attempt.json"))
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

    #[test]
    fn start_or_reuse_attempt_reuses_key_for_same_intent() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let env = TargetEnv::Dev;

        let first = start_or_reuse_attempt(env, "intent-1", 1_000).expect("first attempt");
        let second = start_or_reuse_attempt(env, "intent-1", 1_100).expect("second attempt");

        assert_eq!(first.idempotency_key, second.idempotency_key);
        assert!(!first.reused);
        assert!(second.reused);
    }

    #[test]
    fn start_or_reuse_attempt_rotates_key_for_new_intent() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let env = TargetEnv::Dev;

        let first = start_or_reuse_attempt(env, "intent-1", 1_000).expect("first attempt");
        let second = start_or_reuse_attempt(env, "intent-2", 1_100).expect("second attempt");

        assert_ne!(first.idempotency_key, second.idempotency_key);
        assert!(!second.reused);
    }

    #[test]
    fn submitted_attempt_is_not_reused() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let env = TargetEnv::Dev;

        let first = start_or_reuse_attempt(env, "intent-1", 1_000).expect("first attempt");
        mark_submitted(env, &first.idempotency_key, "order-1", 1_010).expect("mark submitted");

        let second = start_or_reuse_attempt(env, "intent-1", 1_020).expect("second attempt");
        assert_ne!(first.idempotency_key, second.idempotency_key);
        assert!(!second.reused);
    }

    #[test]
    fn mark_failed_sets_failed_status() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let env = TargetEnv::Dev;

        let first = start_or_reuse_attempt(env, "intent-1", 1_000).expect("first attempt");
        mark_failed(env, &first.idempotency_key, "network timeout", 1_050).expect("mark failed");

        let stored = load_attempt(env)
            .expect("load attempt")
            .expect("attempt should exist");
        assert_eq!(stored.status, TradeAttemptStatus::Failed);
        assert_eq!(stored.last_error.as_deref(), Some("network timeout"));
    }

    #[test]
    fn generated_idempotency_key_has_uuid_format() {
        let key = generate_idempotency_key(1_000);
        let bytes = key.as_bytes();

        assert_eq!(bytes.len(), 36);
        assert_eq!(bytes[8], b'-');
        assert_eq!(bytes[13], b'-');
        assert_eq!(bytes[18], b'-');
        assert_eq!(bytes[23], b'-');
        assert!(
            key.chars()
                .enumerate()
                .all(|(idx, ch)| matches!(idx, 8 | 13 | 18 | 23) || ch.is_ascii_hexdigit())
        );
    }

    #[test]
    fn load_recent_submitted_attempt_returns_existing_order_for_same_intent() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let env = TargetEnv::Dev;

        let first = start_or_reuse_attempt(env, "intent-1", 1_000).expect("first attempt");
        mark_submitted(env, &first.idempotency_key, "order-1", 1_010).expect("mark submitted");

        let existing = load_recent_submitted_attempt(env, "intent-1", 1_020)
            .expect("load submitted attempt")
            .expect("submitted attempt should be available");
        assert_eq!(existing.idempotency_key, first.idempotency_key);
        assert_eq!(existing.order_id, "order-1");
    }

    #[test]
    fn load_recent_submitted_attempt_returns_none_when_stale() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let env = TargetEnv::Dev;

        let first = start_or_reuse_attempt(env, "intent-1", 1_000).expect("first attempt");
        mark_submitted(env, &first.idempotency_key, "order-1", 1_010).expect("mark submitted");

        let stale = load_recent_submitted_attempt(env, "intent-1", 1_010 + 3_601)
            .expect("load stale submitted attempt");
        assert!(stale.is_none());
    }

    #[test]
    fn load_recent_submitted_attempt_returns_none_for_other_intent() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let env = TargetEnv::Dev;

        let first = start_or_reuse_attempt(env, "intent-1", 1_000).expect("first attempt");
        mark_submitted(env, &first.idempotency_key, "order-1", 1_010).expect("mark submitted");

        let mismatch = load_recent_submitted_attempt(env, "intent-2", 1_020)
            .expect("load mismatch submitted attempt");
        assert!(mismatch.is_none());
    }

    #[test]
    fn delete_attempt_store_removes_existing_file() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        let _ = start_or_reuse_attempt(TargetEnv::Dev, "intent-1", 1_000).expect("save attempt");

        let path = attempt_file_path().expect("attempt path");
        assert!(path.exists());

        delete_attempt_store().expect("delete attempt store");

        assert!(!path.exists());
    }

    #[test]
    fn delete_attempt_store_ignores_missing_file() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        let path = attempt_file_path().expect("attempt path");
        assert!(!path.exists());

        delete_attempt_store().expect("delete missing attempt store");

        assert!(!path.exists());
    }
}
