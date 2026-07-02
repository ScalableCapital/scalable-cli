use std::fmt::{Display, Formatter};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{
    AppConfig, SessionBackendPreference, TargetEnv, config_dir_path, config_file_display_path,
    ensure_private_dir, write_private_file_atomic,
};

mod keyring_provider;

const KEYRING_SERVICE: &str = "scalable.capital:scalable-cli";
const SESSION_ACCOUNT: &str = "session";
const SESSION_FILENAME: &str = "session.json";
const KEYRING_PROBE_KEY: &str = "__sc_storage_probe__";
const ACTIVE_SESSION_LOCK_FILENAME: &str = "session.lock";

pub const SECRET_STORAGE_UNAVAILABLE_PREFIX: &str =
    "OS secret storage (keyring/Secret Service) is unavailable.";

#[derive(Debug)]
pub enum SessionStorageError {
    SecretStoreUnavailable {
        config_path: Option<PathBuf>,
        source: keyring_core::Error,
    },
}

impl SessionStorageError {
    pub(crate) fn secret_store_unavailable(source: keyring_core::Error) -> Self {
        Self::SecretStoreUnavailable {
            config_path: config_file_display_path().ok(),
            source,
        }
    }
}

impl Display for SessionStorageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SecretStoreUnavailable { config_path, .. } => {
                let location = config_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "your scalable-cli config.toml".to_string());
                write!(
                    f,
                    "{SECRET_STORAGE_UNAVAILABLE_PREFIX} Store the session in a local file instead: add `session_backend = \"file\"` under `[auth]` in {location}, then run 'sc login' again.\n\n[auth]\nsession_backend = \"file\""
                )
            }
        }
    }
}

impl std::error::Error for SessionStorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SecretStoreUnavailable { source, .. } => Some(source),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoginSource {
    DeviceCode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    LocalReadOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub expires_at: Option<i64>,
    pub person_id: String,
    pub source: LoginSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSession {
    pub env: TargetEnv,
    pub session: Session,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dpop_jwk_thumbprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<SessionMode>,
}

pub trait SecretStore {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

#[derive(Default)]
pub struct KeyringStore;

impl KeyringStore {
    fn validate_available(&self) -> Result<()> {
        let entry = keyring_entry(KEYRING_PROBE_KEY)?;
        match entry.get_password() {
            Ok(_) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_keyring_error(err)),
        }
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let entry = keyring_entry(key)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_keyring_error(err)),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        let entry = keyring_entry(key)?;
        entry.set_password(value).map_err(map_keyring_error)?;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let entry = keyring_entry(key)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_keyring_error(err)),
        }
    }
}

pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: PathBuf) -> Result<Self> {
        ensure_private_dir(&dir)?;
        Ok(Self { dir })
    }

    pub fn from_default_path() -> Result<Self> {
        let dir = default_session_dir()?;
        Self::new(dir)
    }

    fn path_for_key(&self, key: &str) -> PathBuf {
        if key == SESSION_ACCOUNT {
            return self.dir.join(SESSION_FILENAME);
        }
        self.dir.join(format!("{key}.json"))
    }
}

impl SecretStore for FileStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let path = self.path_for_key(key);
        if !path.exists() {
            return Ok(None);
        }
        let value = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read session file {}", path.display()))?;
        Ok(Some(value))
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        let path = self.path_for_key(key);
        write_private_file_atomic(&path, value.as_bytes())
            .with_context(|| format!("Failed to write session file {}", path.display()))?;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let path = self.path_for_key(key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err)
                .with_context(|| format!("Failed to delete session file {}", path.display())),
        }
    }
}

pub enum StorageBackend {
    File(FileStore),
    Keyring(KeyringStore),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretWriteBackend {
    File,
    Keyring,
}

#[derive(Debug, Clone)]
pub struct StorageBackendDiagnostics {
    pub configured_backend: String,
    pub effective_backend: String,
    pub fallback_reason: Option<String>,
}

impl StorageBackend {
    pub fn from_config(config: &AppConfig) -> Result<Self> {
        let backend_kind = BackendKind::from_preference(config.auth.session_backend);

        match backend_kind {
            BackendKind::File => Ok(Self::File(FileStore::from_default_path()?)),
            BackendKind::Keyring => Ok(Self::Keyring(KeyringStore)),
        }
    }

    fn set_with_backend(&self, key: &str, value: &str) -> Result<SecretWriteBackend> {
        match self {
            Self::File(store) => {
                store.set(key, value)?;
                Ok(SecretWriteBackend::File)
            }
            Self::Keyring(store) => {
                store.set(key, value)?;
                Ok(SecretWriteBackend::Keyring)
            }
        }
    }

    fn diagnostics(&self) -> StorageBackendDiagnostics {
        match self {
            Self::File(_) => StorageBackendDiagnostics {
                configured_backend: "file".to_string(),
                effective_backend: "file".to_string(),
                fallback_reason: None,
            },
            Self::Keyring(keyring) => {
                let fallback_reason = keyring_probe_error(keyring);
                StorageBackendDiagnostics {
                    configured_backend: "keyring".to_string(),
                    effective_backend: "keyring".to_string(),
                    fallback_reason,
                }
            }
        }
    }

    fn validate_login_storage_ready(&self) -> Result<()> {
        match self {
            Self::File(_) => Ok(()),
            Self::Keyring(store) => store.validate_available(),
        }
    }
}

impl SecretStore for StorageBackend {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match self {
            Self::File(store) => store.get(key),
            Self::Keyring(store) => store.get(key),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.set_with_backend(key, value).map(|_| ())
    }

    fn delete(&self, key: &str) -> Result<()> {
        match self {
            Self::File(store) => store.delete(key),
            Self::Keyring(store) => store.delete(key),
        }
    }
}

pub struct SessionManager<S: SecretStore = StorageBackend> {
    store: S,
}

impl SessionManager<StorageBackend> {
    pub fn new(config: &AppConfig) -> Result<Self> {
        Ok(Self {
            store: StorageBackend::from_config(config)?,
        })
    }

    pub fn save_active_with_backend(
        &mut self,
        stored_session: &StoredSession,
    ) -> Result<SecretWriteBackend> {
        let _lock = ActiveSessionLock::acquire()?;
        let serialized = serde_json::to_string(stored_session)?;
        self.store.set_with_backend(SESSION_ACCOUNT, &serialized)
    }

    pub fn save_active_locked(&mut self, stored_session: &StoredSession) -> Result<()> {
        let _lock = ActiveSessionLock::acquire()?;
        self.save_active_unlocked(stored_session)
    }

    pub fn save_active_if_matches(
        &mut self,
        expected: &StoredSession,
        replacement: &StoredSession,
    ) -> Result<bool> {
        let _lock = ActiveSessionLock::acquire()?;
        match self.load_active_unlocked()? {
            Some(current) if current == *expected => {
                self.save_active_unlocked(replacement)?;
                Ok(true)
            }
            Some(_) | None => Ok(false),
        }
    }

    pub fn delete_active_if_matches(&mut self, expected: &StoredSession) -> Result<bool> {
        let _lock = ActiveSessionLock::acquire()?;
        match self.load_active_unlocked()? {
            Some(current) if current == *expected => {
                self.store.delete(SESSION_ACCOUNT)?;
                Ok(true)
            }
            Some(_) | None => Ok(false),
        }
    }

    pub fn delete_active_locked(&mut self) -> Result<()> {
        let _lock = ActiveSessionLock::acquire()?;
        self.store.delete(SESSION_ACCOUNT)
    }

    pub fn storage_backend_diagnostics(&self) -> StorageBackendDiagnostics {
        self.store.diagnostics()
    }

    pub fn validate_login_storage_ready(&self) -> Result<()> {
        self.store.validate_login_storage_ready()
    }
}

impl<S: SecretStore> SessionManager<S> {
    pub fn with_store(store: S) -> Self {
        Self { store }
    }

    pub fn load_active(&self) -> Result<Option<StoredSession>> {
        self.load_active_unlocked()
    }

    fn load_active_unlocked(&self) -> Result<Option<StoredSession>> {
        let value = match self.store.get(SESSION_ACCOUNT)? {
            Some(v) => v,
            None => return Ok(None),
        };

        let stored = serde_json::from_str::<StoredSession>(&value)
            .context("Stored active session is invalid")?;
        Ok(Some(stored))
    }

    pub fn save_active(&mut self, stored_session: &StoredSession) -> Result<()> {
        self.save_active_unlocked(stored_session)
    }

    fn save_active_unlocked(&mut self, stored_session: &StoredSession) -> Result<()> {
        let serialized = serde_json::to_string(stored_session)?;
        self.store.set(SESSION_ACCOUNT, &serialized)
    }

    pub fn delete_active(&mut self) -> Result<()> {
        self.store.delete(SESSION_ACCOUNT)
    }

    pub fn load_required_active(&self) -> Result<StoredSession> {
        self.load_active()?
            .context("No active session. Run 'sc login'.")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendKind {
    File,
    Keyring,
}

impl BackendKind {
    fn from_preference(pref: SessionBackendPreference) -> Self {
        match pref {
            SessionBackendPreference::Keyring => Self::Keyring,
            SessionBackendPreference::File => Self::File,
        }
    }
}

fn default_session_dir() -> Result<PathBuf> {
    config_dir_path()
}

struct ActiveSessionLock {
    #[allow(dead_code)]
    file: fs::File,
}

impl ActiveSessionLock {
    fn acquire() -> Result<Self> {
        let path = config_dir_path()?.join(ACTIVE_SESSION_LOCK_FILENAME);
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("Failed opening session lock {}", path.display()))?;
        lock_file_exclusive(&file)
            .with_context(|| format!("Failed locking session state {}", path.display()))?;
        Ok(Self { file })
    }
}

#[cfg(unix)]
fn lock_file_exclusive(file: &fs::File) -> Result<()> {
    use std::os::fd::AsRawFd;

    loop {
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc == 0 {
            return Ok(());
        }

        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }

        return Err(err).context("flock LOCK_EX failed");
    }
}

#[cfg(not(unix))]
fn lock_file_exclusive(_file: &fs::File) -> Result<()> {
    Ok(())
}

fn keyring_entry(user: &str) -> Result<keyring_core::Entry> {
    keyring_provider::entry(KEYRING_SERVICE, user).map_err(map_keyring_error)
}

fn keyring_probe_error(keyring: &KeyringStore) -> Option<String> {
    keyring
        .validate_available()
        .err()
        .map(|err| err.to_string())
}

fn map_keyring_error(err: keyring_core::Error) -> anyhow::Error {
    if keyring_error_is_secret_storage_unavailable(&err) {
        return anyhow::Error::new(SessionStorageError::secret_store_unavailable(err));
    }
    err.into()
}

fn keyring_error_is_secret_storage_unavailable(err: &keyring_core::Error) -> bool {
    if matches!(
        err,
        keyring_core::Error::NoStorageAccess(_) | keyring_core::Error::NoDefaultStore
    ) {
        return true;
    }

    #[cfg(target_os = "linux")]
    {
        let detail = match err {
            keyring_core::Error::PlatformFailure(inner) => inner.to_string(),
            _ => return false,
        };

        let lower = detail.to_lowercase();
        lower.contains("org.freedesktop.secrets")
            || lower.contains("was not provided by any .service")
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = err;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemoryStore {
        values: std::sync::Mutex<HashMap<String, String>>,
    }

    impl SecretStore for MemoryStore {
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.values.lock().expect("lock").get(key).cloned())
        }

        fn set(&self, key: &str, value: &str) -> Result<()> {
            self.values
                .lock()
                .expect("lock")
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<()> {
            self.values.lock().expect("lock").remove(key);
            Ok(())
        }
    }

    fn sample_session() -> Session {
        Session {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            id_token: Some("id".to_string()),
            expires_at: Some(1),
            person_id: "person".to_string(),
            source: LoginSource::DeviceCode,
        }
    }

    fn sample_stored_session() -> StoredSession {
        StoredSession {
            env: TargetEnv::Prod,
            session: sample_session(),
            dpop_jwk_thumbprint: Some("thumbprint-1".to_string()),
            mode: None,
        }
    }

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
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
                Some(value) => unsafe {
                    std::env::set_var(self.key, value);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    #[test]
    fn save_and_load_session() {
        let store = MemoryStore::default();
        let mut manager = SessionManager::with_store(store);
        let stored = sample_stored_session();

        manager.save_active(&stored).expect("save");
        let loaded = manager
            .load_active()
            .expect("load")
            .expect("session exists");

        assert_eq!(loaded.env, TargetEnv::Prod);
        assert_eq!(loaded.session.person_id, "person");
        assert_eq!(loaded.session.access_token, "access");
        assert_eq!(loaded.dpop_jwk_thumbprint.as_deref(), Some("thumbprint-1"));
        assert_eq!(loaded.mode, None);
    }

    #[test]
    fn delete_session() {
        let store = MemoryStore::default();
        let mut manager = SessionManager::with_store(store);
        manager.save_active(&sample_stored_session()).expect("save");
        manager.delete_active().expect("delete");
        assert!(manager.load_active().expect("load").is_none());
    }

    #[test]
    fn load_required_active_errors_without_session() {
        let store = MemoryStore::default();
        let manager = SessionManager::with_store(store);
        let err = manager
            .load_required_active()
            .expect_err("missing session should fail");
        assert!(err.to_string().contains("No active session"));
    }

    #[test]
    fn backend_kind_from_preference_accepts_keyring_and_file() {
        assert_eq!(
            BackendKind::from_preference(SessionBackendPreference::Keyring),
            BackendKind::Keyring
        );
        assert_eq!(
            BackendKind::from_preference(SessionBackendPreference::File),
            BackendKind::File
        );
    }

    #[test]
    fn file_store_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(tmp.path().to_path_buf()).expect("file store init");

        store.set(SESSION_ACCOUNT, "{\"a\":1}").expect("set");
        let loaded = store.get(SESSION_ACCOUNT).expect("get");
        assert_eq!(loaded.as_deref(), Some("{\"a\":1}"));
        assert!(tmp.path().join(SESSION_FILENAME).exists());

        store.delete(SESSION_ACCOUNT).expect("delete");
        assert!(
            store
                .get(SESSION_ACCOUNT)
                .expect("get after delete")
                .is_none()
        );
    }

    #[test]
    fn default_session_dir_uses_config_dir_path() {
        let _lock = crate::lock_test_env();
        let _guard = EnvGuard::set("SC_CONFIG_DIR", "/tmp/sc-custom");
        let dir = default_session_dir().expect("resolve dir");
        assert_eq!(dir, PathBuf::from("/tmp/sc-custom"));
    }

    #[test]
    fn backend_kind_maps_from_config_preference() {
        assert_eq!(
            BackendKind::from_preference(SessionBackendPreference::Keyring),
            BackendKind::Keyring
        );
        assert_eq!(
            BackendKind::from_preference(SessionBackendPreference::File),
            BackendKind::File
        );
    }

    #[test]
    fn save_with_backend_reports_file_backend() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut manager = SessionManager {
            store: StorageBackend::File(
                FileStore::new(tmp.path().to_path_buf()).expect("file store"),
            ),
        };

        let backend = manager
            .save_active_with_backend(&sample_stored_session())
            .expect("save");
        assert_eq!(backend, SecretWriteBackend::File);
    }

    #[test]
    fn validate_login_storage_ready_accepts_file_backend() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manager = SessionManager {
            store: StorageBackend::File(
                FileStore::new(tmp.path().to_path_buf()).expect("file store"),
            ),
        };

        manager
            .validate_login_storage_ready()
            .expect("file backend should be ready");
    }

    #[test]
    fn keyring_error_text_matching_detects_secret_service_unavailable() {
        let err = keyring_core::Error::PlatformFailure(Box::new(std::io::Error::other(
            "DBus error: The name org.freedesktop.secrets was not provided by any .service files",
        )));
        #[cfg(target_os = "linux")]
        assert!(keyring_error_is_secret_storage_unavailable(&err));
        #[cfg(not(target_os = "linux"))]
        assert!(!keyring_error_is_secret_storage_unavailable(&err));
    }

    #[test]
    fn keyring_error_text_matching_ignores_unrelated_failures() {
        let err =
            keyring_core::Error::PlatformFailure(Box::new(std::io::Error::other("Access denied")));
        assert!(!keyring_error_is_secret_storage_unavailable(&err));
    }

    #[test]
    fn map_keyring_error_adds_actionable_file_backend_guidance() {
        let err = map_keyring_error(keyring_core::Error::PlatformFailure(Box::new(
            std::io::Error::other(
                "DBus error: The name org.freedesktop.secrets was not provided by any .service files",
            ),
        )));

        #[cfg(target_os = "linux")]
        assert!(
            err.to_string()
                .starts_with(SECRET_STORAGE_UNAVAILABLE_PREFIX)
        );
        #[cfg(target_os = "linux")]
        assert!(err.to_string().contains("session_backend = \"file\""));
        #[cfg(target_os = "linux")]
        assert!(err.to_string().contains("sc login"));
        #[cfg(target_os = "linux")]
        assert!(err.downcast_ref::<SessionStorageError>().is_some());

        #[cfg(not(target_os = "linux"))]
        assert!(err.downcast_ref::<SessionStorageError>().is_none());
        #[cfg(not(target_os = "linux"))]
        assert!(matches!(
            err.downcast_ref::<keyring_core::Error>(),
            Some(keyring_core::Error::PlatformFailure(_))
        ));
    }

    #[test]
    fn keyring_no_default_store_is_treated_as_secret_storage_unavailable() {
        let err = map_keyring_error(keyring_core::Error::NoDefaultStore);
        assert!(
            err.to_string()
                .starts_with(SECRET_STORAGE_UNAVAILABLE_PREFIX)
        );
        assert!(err.downcast_ref::<SessionStorageError>().is_some());
    }

    #[test]
    fn load_active_accepts_legacy_session_without_dpop_thumbprint() {
        let store = MemoryStore::default();
        let manager = SessionManager::with_store(store);
        manager
            .store
            .set(
                SESSION_ACCOUNT,
                r#"{"env":"prod","session":{"access_token":"access","refresh_token":"refresh","id_token":"id","expires_at":1,"person_id":"person","source":"device_code"}}"#,
            )
            .expect("write legacy payload");

        let loaded = manager
            .load_active()
            .expect("load")
            .expect("session exists");

        assert_eq!(loaded.env, TargetEnv::Prod);
        assert_eq!(loaded.dpop_jwk_thumbprint, None);
        assert_eq!(loaded.mode, None);
    }

    #[test]
    fn load_active_accepts_legacy_session_without_mode() {
        let store = MemoryStore::default();
        let manager = SessionManager::with_store(store);
        manager
            .store
            .set(
                SESSION_ACCOUNT,
                r#"{"env":"prod","session":{"access_token":"access","refresh_token":"refresh","id_token":"id","expires_at":1,"person_id":"person","source":"device_code"},"dpop_jwk_thumbprint":"thumbprint-1"}"#,
            )
            .expect("write legacy payload");

        let loaded = manager
            .load_active()
            .expect("load")
            .expect("session exists");

        assert_eq!(loaded.mode, None);
    }

    #[test]
    fn save_and_load_session_preserves_mode() {
        let store = MemoryStore::default();
        let mut manager = SessionManager::with_store(store);
        let stored = StoredSession {
            mode: Some(SessionMode::LocalReadOnly),
            ..sample_stored_session()
        };

        manager.save_active(&stored).expect("save");
        let loaded = manager
            .load_active()
            .expect("load")
            .expect("session exists");

        assert_eq!(loaded.mode, Some(SessionMode::LocalReadOnly));
    }
}
