use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::config::{config_dir_path, set_private_file_permissions};

const INSTALLATION_CODE_FILE_NAME: &str = "installation_code.json";
const CANONICAL_CODE_LENGTH: usize = 16;
const RAW_CODE_BYTES: usize = 10;
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationCodeValue {
    pub installation_code: String,
    pub display_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredInstallationCode {
    code: String,
}

type LoadStoredInstallationCodeResult =
    std::result::Result<Option<StoredInstallationCode>, StoredInstallationCodeLoadError>;

pub fn load_or_create_installation_code() -> Result<InstallationCodeValue> {
    let path = installation_code_file_path()?;
    let stored = match load_stored_installation_code(&path) {
        Ok(Some(stored)) => stored,
        Ok(None) => create_or_load_stored_installation_code(&path)?,
        Err(StoredInstallationCodeLoadError::InvalidState(err)) => {
            return Err(invalid_installation_code_state_error(&path, err));
        }
        Err(StoredInstallationCodeLoadError::Io(err)) => return Err(err),
    };

    validate_canonical_code(&stored.code)
        .map_err(|err| invalid_installation_code_state_error(&path, err))?;

    Ok(InstallationCodeValue {
        display_code: format_display_code(&stored.code)?,
        installation_code: stored.code,
    })
}

pub fn installation_code_file_path() -> Result<PathBuf> {
    Ok(config_dir_path()?.join(INSTALLATION_CODE_FILE_NAME))
}

pub fn validate_canonical_code(code: &str) -> Result<()> {
    if code.len() != CANONICAL_CODE_LENGTH {
        return Err(anyhow!(
            "expected {} uppercase base32 characters",
            CANONICAL_CODE_LENGTH
        ));
    }

    if !code
        .bytes()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'2'..=b'7'))
    {
        return Err(anyhow!(
            "expected only uppercase base32 characters in the A-Z2-7 alphabet"
        ));
    }

    Ok(())
}

pub fn format_display_code(code: &str) -> Result<String> {
    validate_canonical_code(code)?;
    let mut display = String::with_capacity(CANONICAL_CODE_LENGTH + 3);
    for (index, chunk) in code.as_bytes().chunks(4).enumerate() {
        if index > 0 {
            display.push('-');
        }
        display.push_str(std::str::from_utf8(chunk).expect("base32 code is ASCII"));
    }
    Ok(display)
}

fn load_stored_installation_code(path: &Path) -> LoadStoredInstallationCodeResult {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(StoredInstallationCodeLoadError::Io(
                anyhow::Error::new(err).context(format!(
                    "Failed to read installation code file at {}",
                    path.display()
                )),
            ));
        }
    };
    parse_stored_installation_code(path, &raw)
}

fn create_or_load_stored_installation_code(path: &Path) -> Result<StoredInstallationCode> {
    let parent = path
        .parent()
        .context("Installation code path is missing a parent directory")?;
    let code = generate_canonical_code();
    let stored = StoredInstallationCode { code };
    let mut temp_file = tempfile::NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "Failed to create temporary installation code file in {}",
            parent.display()
        )
    })?;
    let temp_path = temp_file.path().to_path_buf();
    write_stored_installation_code(temp_file.as_file_mut(), &temp_path, &stored)?;

    match temp_file.persist_noclobber(path) {
        Ok(_) => {
            set_private_file_permissions(path)?;
            Ok(stored)
        }
        Err(err) if err.error.kind() == std::io::ErrorKind::AlreadyExists => {
            match load_stored_installation_code(path) {
                Ok(Some(stored)) => Ok(stored),
                Ok(None) => Err(anyhow!(
                    "Installation code file at {} disappeared before it could be loaded",
                    path.display()
                )),
                Err(StoredInstallationCodeLoadError::InvalidState(err)) => {
                    Err(invalid_installation_code_state_error(path, err))
                }
                Err(StoredInstallationCodeLoadError::Io(err)) => Err(err),
            }
        }
        Err(err) => Err(err.error).with_context(|| {
            format!(
                "Failed to persist installation code file at {}",
                path.display()
            )
        }),
    }
}

fn parse_stored_installation_code(path: &Path, raw: &str) -> LoadStoredInstallationCodeResult {
    serde_json::from_str::<StoredInstallationCode>(raw)
        .map(Some)
        .map_err(|err| {
            StoredInstallationCodeLoadError::InvalidState(anyhow::Error::new(err).context(format!(
                "Invalid installation code JSON at {}",
                path.display()
            )))
        })
}

fn write_stored_installation_code(
    file: &mut fs::File,
    path: &Path,
    stored: &StoredInstallationCode,
) -> Result<()> {
    let serialized = serde_json::to_string_pretty(stored)?;
    file.write_all(serialized.as_bytes()).with_context(|| {
        format!(
            "Failed to write installation code file at {}",
            path.display()
        )
    })?;
    file.flush().with_context(|| {
        format!(
            "Failed to flush installation code file at {}",
            path.display()
        )
    })?;
    set_private_file_permissions(path)?;
    Ok(())
}

enum StoredInstallationCodeLoadError {
    InvalidState(anyhow::Error),
    Io(anyhow::Error),
}

fn invalid_installation_code_state_error(
    path: &Path,
    detail: impl std::fmt::Display,
) -> anyhow::Error {
    anyhow!(
        "Invalid installation code state at {}: {}. Delete {} and rerun `sc installation-code`.",
        path.display(),
        detail,
        path.display()
    )
}

fn generate_canonical_code() -> String {
    let mut bytes = [0_u8; RAW_CODE_BYTES];
    let mut rng = rand::rng();
    rng.fill(&mut bytes);
    encode_base32_uppercase(&bytes)
}

fn encode_base32_uppercase(bytes: &[u8; RAW_CODE_BYTES]) -> String {
    let mut output = String::with_capacity(CANONICAL_CODE_LENGTH);
    let mut buffer = 0_u32;
    let mut bits_left = 0_u8;

    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits_left += 8;

        while bits_left >= 5 {
            let shift = bits_left - 5;
            let index = ((buffer >> shift) & 0b1_1111) as usize;
            output.push(BASE32_ALPHABET[index] as char);
            bits_left -= 5;
        }
    }

    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0b1_1111) as usize;
        output.push(BASE32_ALPHABET[index] as char);
    }

    output
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
    fn format_display_code_groups_canonical_code() {
        assert_eq!(
            format_display_code("ABCDEFGHIJKLMNOP").expect("display code"),
            "ABCD-EFGH-IJKL-MNOP"
        );
    }

    #[test]
    fn validate_canonical_code_rejects_invalid_values() {
        assert_eq!(
            validate_canonical_code("abcd")
                .expect_err("short code should fail")
                .to_string(),
            "expected 16 uppercase base32 characters"
        );
        assert!(validate_canonical_code("ABCDEFGHIJKLMNOPQ").is_err());
        assert!(validate_canonical_code("ABCDEF89IJKLMNOP").is_err());
    }

    #[test]
    fn encode_base32_uppercase_matches_reference_vectors() {
        assert_eq!(
            encode_base32_uppercase(&[0_u8; RAW_CODE_BYTES]),
            "AAAAAAAAAAAAAAAA"
        );
        assert_eq!(
            encode_base32_uppercase(&[0xff_u8; RAW_CODE_BYTES]),
            "7777777777777777"
        );
        assert_eq!(
            encode_base32_uppercase(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc]),
            "AERUKZ4JVPG677W4"
        );
    }

    #[test]
    fn load_or_create_reuses_existing_code() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);

        let first = load_or_create_installation_code().expect("first load");
        let second = load_or_create_installation_code().expect("second load");

        assert_eq!(first.installation_code, second.installation_code);
        assert_eq!(first.display_code, second.display_code);
        assert_eq!(first.display_code.len(), 19);
        assert!(
            first
                .installation_code
                .bytes()
                .all(|byte| matches!(byte, b'A'..=b'Z' | b'2'..=b'7'))
        );
    }

    #[test]
    fn create_or_load_returns_existing_code_when_file_already_exists() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let path = installation_code_file_path().expect("path");
        fs::write(&path, r#"{ "code": "ABCDEFGHIJKLMNOP" }"#).expect("write code");

        let stored = create_or_load_stored_installation_code(&path)
            .expect("existing file should be loaded instead of overwritten");

        assert_eq!(stored.code, "ABCDEFGHIJKLMNOP");
        assert_eq!(
            fs::read_to_string(&path).expect("persisted code"),
            r#"{ "code": "ABCDEFGHIJKLMNOP" }"#
        );
    }

    #[test]
    fn load_or_create_scopes_to_config_dir() {
        let _lock = crate::lock_test_env();
        let (_tmp_one, config_dir_one) = temp_config_dir();
        let _guard_one = EnvGuard::set("SC_CONFIG_DIR", config_dir_one);
        let first = load_or_create_installation_code().expect("first code");

        let (_tmp_two, config_dir_two) = temp_config_dir();
        let _guard_two = EnvGuard::set("SC_CONFIG_DIR", config_dir_two);
        let second = load_or_create_installation_code().expect("second code");

        assert_ne!(first.installation_code, second.installation_code);
    }

    #[test]
    fn invalid_state_reports_manual_reset_guidance() {
        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let path = installation_code_file_path().expect("path");
        fs::write(&path, r#"{ "code": "bad" }"#).expect("write invalid code");

        let err = load_or_create_installation_code()
            .expect_err("invalid persisted code should fail")
            .to_string();

        assert!(err.contains("Delete"));
        assert!(err.contains("sc installation-code"));
    }

    #[cfg(unix)]
    #[test]
    fn read_errors_do_not_report_invalid_state_guidance() {
        use std::os::unix::fs::PermissionsExt;

        let _lock = crate::lock_test_env();
        let (_tmp, config_dir) = temp_config_dir();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", config_dir);
        let path = installation_code_file_path().expect("path");
        fs::write(&path, r#"{ "code": "ABCDEFGHIJKLMNOP" }"#).expect("write code");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000))
            .expect("remove read permissions");

        let err = load_or_create_installation_code()
            .expect_err("unreadable persisted code should fail")
            .to_string();

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("restore permissions");

        assert!(err.contains("Failed to read installation code file"));
        assert!(!err.contains("Invalid installation code state"));
        assert!(!err.contains("Delete"));
    }
}
