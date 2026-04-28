use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{config_dir_path, write_private_file_atomic};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrokerContext {
    #[serde(default)]
    pub account_id: String,
    pub portfolio_id: Option<String>,
}

pub fn load_context() -> Result<Option<BrokerContext>> {
    let path = context_file_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read broker context at {}", path.display()))?;
    let context = serde_json::from_str::<BrokerContext>(&raw)
        .with_context(|| format!("Invalid broker context JSON at {}", path.display()))?;
    Ok(Some(context))
}

pub fn save_context(context: BrokerContext) -> Result<()> {
    let path = context_file_path()?;
    let serialized = serde_json::to_string_pretty(&context)?;
    write_private_file_atomic(&path, serialized.as_bytes())
        .with_context(|| format!("Failed to write broker context at {}", path.display()))?;
    Ok(())
}

pub fn delete_context() -> Result<()> {
    let path = context_file_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("Failed to delete broker context at {}", path.display())),
    }
}

pub fn context_file_path() -> Result<PathBuf> {
    Ok(config_dir_path()?.join("broker_context.json"))
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
    fn save_and_load_context_as_single_entry() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        save_context(BrokerContext {
            account_id: "acc-dev".to_string(),
            portfolio_id: Some("port-dev".to_string()),
        })
        .expect("save context");

        let context = load_context().expect("load").expect("ctx");

        assert_eq!(context.account_id, "acc-dev");
        assert_eq!(context.portfolio_id.as_deref(), Some("port-dev"));

        let raw = fs::read_to_string(context_file_path().expect("path")).expect("read context");
        assert!(raw.contains("\"account_id\""));
    }

    #[test]
    fn load_context_allows_missing_account_id_for_single_format() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let path = context_file_path().expect("context path");
        fs::write(&path, r#"{ "portfolio_id": "legacy-portfolio" }"#)
            .expect("write compatibility context");

        let context = load_context().expect("load").expect("context");

        assert!(context.account_id.is_empty());
        assert_eq!(context.portfolio_id.as_deref(), Some("legacy-portfolio"));
    }

    #[test]
    fn delete_context_removes_existing_file() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        save_context(BrokerContext {
            account_id: "acc-dev".to_string(),
            portfolio_id: Some("port-dev".to_string()),
        })
        .expect("save context");

        let path = context_file_path().expect("context path");
        assert!(path.exists());

        delete_context().expect("delete context");

        assert!(!path.exists());
    }

    #[test]
    fn delete_context_ignores_missing_file() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        let path = context_file_path().expect("context path");
        assert!(!path.exists());

        delete_context().expect("delete missing context");

        assert!(!path.exists());
    }
}
