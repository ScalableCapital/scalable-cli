use std::fmt::{Display, Formatter};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
const ENV_XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
#[cfg(unix)]
const PASSWD_BUFFER_FALLBACK_LEN: usize = 512;
#[cfg(unix)]
const PASSWD_BUFFER_MAX_LEN: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetEnv {
    Dev,
    Prod,
}

impl TargetEnv {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Prod => "prod",
        }
    }
}

impl Display for TargetEnv {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TargetEnv {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "dev" => Ok(Self::Dev),
            "prod" => Ok(Self::Prod),
            other => Err(anyhow!("Invalid environment '{other}'. Use dev or prod.")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionBackendPreference {
    #[cfg_attr(any(target_os = "macos", target_os = "linux"), default)]
    Keyring,
    #[cfg_attr(not(any(target_os = "macos", target_os = "linux")), default)]
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeAuthConfig {
    #[serde(default)]
    pub session_backend: SessionBackendPreference,
    #[serde(default = "default_signing_key_backend")]
    pub signing_key_backend: DpopKeyBackend,
    #[serde(default)]
    pub pkcs11: Option<Pkcs11Config>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DpopKeyBackend {
    #[default]
    File,
    SecureEnclave,
    Pkcs11,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pkcs11Config {
    pub module_path: String,
    pub key_uri: String,
}

impl Default for RuntimeAuthConfig {
    fn default() -> Self {
        Self {
            session_backend: SessionBackendPreference::default(),
            signing_key_backend: default_signing_key_backend(),
            pkcs11: None,
        }
    }
}

const fn default_signing_key_backend() -> DpopKeyBackend {
    #[cfg(target_os = "macos")]
    {
        DpopKeyBackend::SecureEnclave
    }

    #[cfg(not(target_os = "macos"))]
    {
        DpopKeyBackend::File
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default)]
    pub auth: RuntimeAuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvConfig {
    pub graphql_url: String,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub issuer: String,
    pub audience: String,
    pub client_id: String,
}

impl AppConfig {
    pub fn load_or_default() -> Result<Self> {
        let path = config_file_path()?;
        if path.exists() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read config at {}", path.display()))?;
            let cfg = toml::from_str::<AppConfig>(&raw)
                .with_context(|| format!("Invalid config TOML at {}", path.display()))?;
            return Ok(cfg);
        }

        Ok(Self::default())
    }
}

pub fn config_dir_path() -> Result<PathBuf> {
    let dir = default_config_dir_path()?;
    ensure_private_dir(&dir)?;
    Ok(dir)
}

pub fn config_file_path() -> Result<PathBuf> {
    Ok(config_dir_path()?.join("config.toml"))
}

pub fn config_file_display_path() -> Result<PathBuf> {
    Ok(default_config_dir_path()?.join("config.toml"))
}

pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("Failed creating dir {}", dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).with_context(|| {
            format!("Failed setting directory permissions on {}", dir.display())
        })?;
    }

    Ok(())
}

pub fn set_private_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed setting file permissions on {}", path.display()))?;
    }

    Ok(())
}

pub fn write_private_file_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    write_private_file_atomic_with(path, |file| {
        file.write_all(contents)
            .with_context(|| format!("Failed writing temporary file for {}", path.display()))
    })
}

fn write_private_file_atomic_with<F>(path: &Path, write: F) -> Result<()>
where
    F: FnOnce(&mut fs::File) -> Result<()>,
{
    let parent = path
        .parent()
        .context("Atomic write path is missing a parent directory")?;
    ensure_private_dir(parent)?;
    reject_non_regular_existing_path(path)?;

    let mut temp_file = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed creating temporary file in {}", parent.display()))?;
    let temp_path = temp_file.path().to_path_buf();
    set_private_file_permissions(&temp_path)?;

    write(temp_file.as_file_mut())?;
    temp_file
        .as_file_mut()
        .sync_all()
        .with_context(|| format!("Failed syncing temporary file {}", temp_path.display()))?;

    temp_file
        .persist(path)
        .map_err(|err| anyhow::Error::new(err.error))
        .with_context(|| format!("Failed persisting file at {}", path.display()))?;
    set_private_file_permissions(path)?;
    sync_parent_dir(parent)?;
    Ok(())
}

fn reject_non_regular_existing_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(anyhow!(
            "Refusing to overwrite non-regular file at {}",
            path.display()
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => {
            Err(err).with_context(|| format!("Failed inspecting existing path {}", path.display()))
        }
    }
}

#[cfg(unix)]
fn sync_parent_dir(dir: &Path) -> Result<()> {
    let dir_handle = fs::File::open(dir)
        .with_context(|| format!("Failed opening parent directory {}", dir.display()))?;

    match dir_handle.sync_all() {
        Ok(()) => Ok(()),
        // Some Unix filesystems reject directory fsync even after the rename succeeded.
        Err(err) if is_unsupported_directory_sync_error(&err) => Ok(()),
        Err(err) => {
            Err(err).with_context(|| format!("Failed syncing parent directory {}", dir.display()))
        }
    }
}

#[cfg(unix)]
fn is_unsupported_directory_sync_error(err: &std::io::Error) -> bool {
    const EINVAL: i32 = 22;
    const ENOTSUP_DARWIN: i32 = 45;
    const EOPNOTSUPP_LINUX: i32 = 95;

    matches!(
        err.raw_os_error(),
        Some(EINVAL | ENOTSUP_DARWIN | EOPNOTSUPP_LINUX)
    )
}

#[cfg(not(unix))]
fn sync_parent_dir(_dir: &Path) -> Result<()> {
    Ok(())
}

fn default_config_dir_path() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    {
        if let Ok(raw_dir) = std::env::var("SC_CONFIG_DIR") {
            let trimmed = raw_dir.trim();
            if !trimmed.is_empty() {
                return Ok(PathBuf::from(trimmed));
            }
        }
    }

    let home_dir = default_home_dir_path()?;
    #[cfg(target_os = "linux")]
    {
        Ok(linux_default_config_dir_path(
            &home_dir,
            std::env::var(ENV_XDG_CONFIG_HOME).ok().as_deref(),
        ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(home_dir.join(".config").join("scalable-cli"))
    }
}

fn default_home_dir_path() -> Result<PathBuf> {
    home_dir_from_env()
        .or_else(home_dir_from_os)
        .context("Could not resolve home directory for default config path")
}

fn home_dir_from_env() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(unix)]
fn home_dir_from_os() -> Option<PathBuf> {
    use std::ffi::{CStr, OsString};
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStringExt;
    use std::ptr;

    let initial_buffer_len = {
        // SAFETY: sysconf reads a process-global configuration value and does
        // not require any additional invariants from Rust.
        unsafe { clamp_passwd_buffer_len(libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX)) }
    };

    lookup_home_dir_from_passwd_with(initial_buffer_len, |buffer_len| unsafe {
        // SAFETY: we call the standard passwd lookup functions with a writable
        // buffer and only read the returned home directory when libc reports
        // success with a non-null result pointer.
        let mut buffer = vec![0_u8; buffer_len];
        let mut passwd = MaybeUninit::<libc::passwd>::uninit();
        let mut result = ptr::null_mut();
        let status = libc::getpwuid_r(
            libc::getuid(),
            passwd.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        );

        if status != 0 {
            return Err(status);
        }

        if result.is_null() {
            return Ok(None);
        }

        let passwd = passwd.assume_init();
        if passwd.pw_dir.is_null() {
            return Ok(None);
        }

        let home = CStr::from_ptr(passwd.pw_dir).to_bytes();
        if home.is_empty() {
            Ok(None)
        } else {
            Ok(Some(PathBuf::from(OsString::from_vec(home.to_vec()))))
        }
    })
}

#[cfg(unix)]
fn clamp_passwd_buffer_len(raw_len: libc::c_long) -> usize {
    if raw_len <= 0 {
        PASSWD_BUFFER_FALLBACK_LEN
    } else {
        usize::try_from(raw_len)
            .unwrap_or(PASSWD_BUFFER_MAX_LEN)
            .clamp(PASSWD_BUFFER_FALLBACK_LEN, PASSWD_BUFFER_MAX_LEN)
    }
}

#[cfg(unix)]
fn grow_passwd_buffer_len(current_len: usize) -> Option<usize> {
    if current_len >= PASSWD_BUFFER_MAX_LEN {
        None
    } else {
        Some(
            current_len
                .saturating_mul(2)
                .clamp(PASSWD_BUFFER_FALLBACK_LEN, PASSWD_BUFFER_MAX_LEN),
        )
    }
}

#[cfg(unix)]
fn lookup_home_dir_from_passwd_with<F>(initial_buffer_len: usize, mut lookup: F) -> Option<PathBuf>
where
    F: FnMut(usize) -> std::result::Result<Option<PathBuf>, i32>,
{
    let mut buffer_len = initial_buffer_len;

    loop {
        match lookup(buffer_len) {
            Ok(result) => return result,
            Err(libc::ERANGE) => {
                buffer_len = grow_passwd_buffer_len(buffer_len)?;
            }
            Err(_) => return None,
        }
    }
}

#[cfg(not(unix))]
fn home_dir_from_os() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "linux")]
fn linux_default_config_dir_path(home_dir: &Path, xdg_config_home: Option<&str>) -> PathBuf {
    if let Some(raw_xdg_config_home) = xdg_config_home {
        let trimmed = raw_xdg_config_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("scalable-cli");
        }
    }

    home_dir.join(".config").join("scalable-cli")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::io::Write;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    #[cfg(unix)]
    use std::{io, io::ErrorKind};
    use tempfile::TempDir;

    fn temp_config_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    struct EnvGuard {
        key: &'static str,
        old_value: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old_value = std::env::var_os(key);
            // SAFETY: config tests serialize environment mutation via
            // crate::lock_test_env and restore the previous value on drop.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old_value }
        }

        fn unset(key: &'static str) -> Self {
            let old_value = std::env::var_os(key);
            // SAFETY: config tests serialize environment mutation via
            // crate::lock_test_env and restore the previous value on drop.
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, old_value }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.old_value {
                // SAFETY: config tests serialize environment mutation via
                // crate::lock_test_env and restore the previous value on drop.
                unsafe {
                    std::env::set_var(self.key, value);
                }
            } else {
                // SAFETY: config tests serialize environment mutation via
                // crate::lock_test_env and restore the previous value on drop.
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[test]
    fn env_parse_works() {
        assert_eq!(TargetEnv::from_str("dev").unwrap(), TargetEnv::Dev);
        assert_eq!(TargetEnv::from_str("prod").unwrap(), TargetEnv::Prod);
        assert!(TargetEnv::from_str("stage").is_err());
    }

    #[test]
    fn default_session_backend_matches_platform_policy() {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert_eq!(
            RuntimeAuthConfig::default().session_backend,
            SessionBackendPreference::Keyring
        );

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        assert_eq!(
            RuntimeAuthConfig::default().session_backend,
            SessionBackendPreference::File
        );
    }

    #[test]
    fn default_signing_key_backend_matches_platform_policy() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            RuntimeAuthConfig::default().signing_key_backend,
            DpopKeyBackend::SecureEnclave
        );

        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            RuntimeAuthConfig::default().signing_key_backend,
            DpopKeyBackend::File
        );
    }

    #[test]
    fn default_home_dir_path_prefers_home_env() {
        let _lock = crate::lock_test_env();
        let _home_guard = EnvGuard::set("HOME", "/tmp/sc-home");

        assert_eq!(
            default_home_dir_path().unwrap(),
            PathBuf::from("/tmp/sc-home")
        );
    }

    #[cfg(unix)]
    #[test]
    fn default_home_dir_path_falls_back_to_passwd_when_home_missing() {
        let _lock = crate::lock_test_env();
        let _home_guard = EnvGuard::unset("HOME");

        let home = default_home_dir_path().expect("passwd fallback should resolve home directory");
        assert!(!home.as_os_str().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn clamp_passwd_buffer_len_uses_fallback_and_cap() {
        assert_eq!(clamp_passwd_buffer_len(-1), PASSWD_BUFFER_FALLBACK_LEN);
        assert_eq!(clamp_passwd_buffer_len(0), PASSWD_BUFFER_FALLBACK_LEN);
        assert_eq!(
            clamp_passwd_buffer_len((PASSWD_BUFFER_MAX_LEN as libc::c_long) + 1),
            PASSWD_BUFFER_MAX_LEN
        );
    }

    #[cfg(unix)]
    #[test]
    fn grow_passwd_buffer_len_doubles_until_cap() {
        assert_eq!(
            grow_passwd_buffer_len(PASSWD_BUFFER_FALLBACK_LEN),
            Some(1024)
        );
        assert_eq!(
            grow_passwd_buffer_len(PASSWD_BUFFER_MAX_LEN / 2),
            Some(PASSWD_BUFFER_MAX_LEN)
        );
        assert_eq!(grow_passwd_buffer_len(PASSWD_BUFFER_MAX_LEN), None);
    }

    #[cfg(unix)]
    #[test]
    fn lookup_home_dir_from_passwd_retries_on_erange() {
        let mut attempts = Vec::new();

        let result = lookup_home_dir_from_passwd_with(PASSWD_BUFFER_FALLBACK_LEN, |buffer_len| {
            attempts.push(buffer_len);
            if attempts.len() == 1 {
                Err(libc::ERANGE)
            } else {
                Ok(Some(PathBuf::from("/tmp/passwd-home")))
            }
        });

        assert_eq!(result, Some(PathBuf::from("/tmp/passwd-home")));
        assert_eq!(attempts, vec![PASSWD_BUFFER_FALLBACK_LEN, 1024]);
    }

    #[cfg(unix)]
    #[test]
    fn lookup_home_dir_from_passwd_stops_after_max_buffer() {
        let mut attempts = Vec::new();

        let result = lookup_home_dir_from_passwd_with(PASSWD_BUFFER_MAX_LEN, |buffer_len| {
            attempts.push(buffer_len);
            Err(libc::ERANGE)
        });

        assert_eq!(result, None);
        assert_eq!(attempts, vec![PASSWD_BUFFER_MAX_LEN]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_default_config_dir_prefers_xdg_config_home_when_set() {
        let dir = linux_default_config_dir_path(
            Path::new("/home/test-user"),
            Some("/var/lib/test-user/.config"),
        );
        assert_eq!(
            dir,
            PathBuf::from("/var/lib/test-user/.config/scalable-cli")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_default_config_dir_falls_back_when_xdg_config_home_empty() {
        let dir = linux_default_config_dir_path(Path::new("/home/test-user"), Some("   "));
        assert_eq!(dir, PathBuf::from("/home/test-user/.config/scalable-cli"));
    }

    #[test]
    fn app_config_parses_auth_backends() {
        let raw = r#"
[auth]
session_backend = "file"
signing_key_backend = "secure_enclave"
"#;
        let cfg = toml::from_str::<AppConfig>(raw).expect("auth config should parse");
        assert_eq!(cfg.auth.session_backend, SessionBackendPreference::File);
        assert_eq!(cfg.auth.signing_key_backend, DpopKeyBackend::SecureEnclave);
        assert_eq!(cfg.auth.pkcs11, None);
    }

    #[test]
    fn app_config_parses_pkcs11_auth_config() {
        let raw = r#"
[auth]
session_backend = "file"
signing_key_backend = "pkcs11"

[auth.pkcs11]
module_path = "/usr/lib/opensc-pkcs11.so"
key_uri = "pkcs11:token=YubiKey%20PIV;id=%01"
"#;
        let cfg = toml::from_str::<AppConfig>(raw).expect("pkcs11 auth config should parse");

        assert_eq!(cfg.auth.session_backend, SessionBackendPreference::File);
        assert_eq!(cfg.auth.signing_key_backend, DpopKeyBackend::Pkcs11);
        assert_eq!(
            cfg.auth.pkcs11,
            Some(Pkcs11Config {
                module_path: "/usr/lib/opensc-pkcs11.so".to_string(),
                key_uri: "pkcs11:token=YubiKey%20PIV;id=%01".to_string(),
            })
        );
    }

    #[test]
    fn app_config_uses_platform_default_signing_backend_when_auth_field_is_missing() {
        let raw = r#"
[auth]
session_backend = "file"
"#;
        let cfg = toml::from_str::<AppConfig>(raw).expect("partial auth config should parse");

        assert_eq!(cfg.auth.session_backend, SessionBackendPreference::File);
        assert_eq!(cfg.auth.signing_key_backend, default_signing_key_backend());
        assert_eq!(cfg.auth.pkcs11, None);
    }

    #[test]
    fn app_config_rejects_unknown_top_level_keys() {
        let raw = r#"
foo = "bar"
"#;
        let err = toml::from_str::<AppConfig>(raw).expect_err("unknown keys should fail");
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn config_file_display_path_uses_override_without_creating_dir() {
        let _lock = crate::lock_test_env();
        let tmp = temp_config_dir();
        let override_dir = tmp.path().join("custom-config");
        let _guard = EnvGuard::set("SC_CONFIG_DIR", override_dir.to_string_lossy().as_ref());

        let path = config_file_display_path().expect("display path");

        assert_eq!(path, override_dir.join("config.toml"));
        assert!(!override_dir.exists());
    }

    #[test]
    fn app_config_rejects_unknown_auth_keys() {
        let raw = r#"
[auth]
auto = true
"#;
        let err = toml::from_str::<AppConfig>(raw).expect_err("unknown keys should fail");
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn app_config_rejects_unknown_pkcs11_keys() {
        let raw = r#"
[auth]
signing_key_backend = "pkcs11"

[auth.pkcs11]
module_path = "/usr/lib/opensc-pkcs11.so"
key_uri = "pkcs11:token=YubiKey%20PIV;id=%01"
slot = "0"
"#;
        let err = toml::from_str::<AppConfig>(raw).expect_err("unknown keys should fail");
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn app_config_rejects_incomplete_pkcs11_config() {
        let raw = r#"
[auth]
signing_key_backend = "pkcs11"

[auth.pkcs11]
module_path = "/usr/lib/opensc-pkcs11.so"
"#;
        let err = toml::from_str::<AppConfig>(raw).expect_err("missing key_uri should fail");
        assert!(err.to_string().contains("missing field `key_uri`"));
    }

    #[test]
    fn atomic_private_write_replaces_existing_file_contents() {
        let tmp = temp_config_dir();
        let path = tmp.path().join("state.json");
        fs::write(&path, "old").expect("seed file");

        write_private_file_atomic(&path, br#"{"state":"new"}"#).expect("write");

        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            r#"{"state":"new"}"#
        );
    }

    #[test]
    fn atomic_private_write_preserves_existing_file_when_write_fails() {
        let tmp = temp_config_dir();
        let path = tmp.path().join("state.json");
        fs::write(&path, "old").expect("seed file");

        let err = write_private_file_atomic_with(&path, |file| {
            file.write_all(b"partial").expect("write partial");
            Err(anyhow!("simulated failure"))
        })
        .expect_err("write should fail");

        assert!(err.to_string().contains("simulated failure"));
        assert_eq!(fs::read_to_string(&path).expect("read"), "old");
    }

    #[test]
    fn atomic_private_write_rejects_directory_targets() {
        let tmp = temp_config_dir();
        let path = tmp.path().join("state");
        fs::create_dir(&path).expect("create dir");

        let err = write_private_file_atomic(&path, br#"{"state":"new"}"#).expect_err("reject");

        assert!(
            err.to_string()
                .contains("Refusing to overwrite non-regular file")
        );
        assert!(path.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_private_write_rejects_symlink_targets() {
        let tmp = temp_config_dir();
        let real_path = tmp.path().join("real.json");
        let symlink_path = tmp.path().join("state.json");
        fs::write(&real_path, "real").expect("seed real file");
        symlink(&real_path, &symlink_path).expect("create symlink");

        let err =
            write_private_file_atomic(&symlink_path, br#"{"state":"new"}"#).expect_err("reject");

        assert!(
            err.to_string()
                .contains("Refusing to overwrite non-regular file")
        );
        assert_eq!(fs::read_to_string(&real_path).expect("read real"), "real");
        assert!(
            fs::symlink_metadata(&symlink_path)
                .expect("symlink metadata")
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn unsupported_directory_sync_errno_values_are_tolerated() {
        assert!(is_unsupported_directory_sync_error(
            &io::Error::from_raw_os_error(22)
        ));
        assert!(is_unsupported_directory_sync_error(
            &io::Error::from_raw_os_error(45)
        ));
        assert!(is_unsupported_directory_sync_error(
            &io::Error::from_raw_os_error(95)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn ordinary_directory_sync_errors_are_not_tolerated() {
        assert!(!is_unsupported_directory_sync_error(
            &io::Error::from_raw_os_error(1)
        ));
        assert!(!is_unsupported_directory_sync_error(&io::Error::new(
            ErrorKind::PermissionDenied,
            "denied",
        )));
    }
}
