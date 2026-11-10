use anyhow::Result;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use mockito::{Matcher, Mock, Server};
use serde_json::json;
use std::ffi::OsString;
use std::io::{Read, Write};
use tempfile::tempdir;

use super::*;
use crate::auth::REFRESH_RETRYABLE_PREFIX;
use crate::config::{
    AppConfig, AuthConfig, DpopKeyBackend, EnvConfig, RuntimeAuthConfig, SessionBackendPreference,
};
use crate::session::{FileStore, LoginSource, SecretStore, SessionMode, StorageBackend};

const TEST_KID: &str = "test-kid";
const TEST_RSA_N: &str = "0hw4qSyPOM59HeFtuc2a4X-rWKPeAXmvtNHV86mYXh8TWEGc68MPq3V7NYUN-ZHulHPl2FQAMVFt-jWOQy4nJEPp1ak-P0_3Vn3Tc4lwDJXuIBC8RHPG8XMUTj7QsZPM-pf_TO_JyLrlItRZ_xADF6b_fwOhOnK5_UXLm0ZqfCYfMGFXB9Oag_Xv5yHqrHlpTkn-zYSmsC9Nk_FOsN1Apsa1Kfwj9SvRplAm86Azc3obHjHkdiOKximtWJWGi-a5eq3fYRk_csIIxO0DHGYk9rUlUQUiwr_MqJqSVc_xvi6w8j-0_Ca34BsonihOdGrNuA1yzll51pxywREMc07JkQ";
const TEST_RSA_E: &str = "AQAB";
const TEST_PRIVATE_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDSHDipLI84zn0d
4W25zZrhf6tYo94Bea+00dXzqZheHxNYQZzrww+rdXs1hQ35ke6Uc+XYVAAxUW36
NY5DLickQ+nVqT4/T/dWfdNziXAMle4gELxEc8bxcxROPtCxk8z6l/9M78nIuuUi
1Fn/EAMXpv9/A6E6crn9RcubRmp8Jh8wYVcH05qD9e/nIeqseWlOSf7NhKawL02T
8U6w3UCmxrUp/CP1K9GmUCbzoDNzehseMeR2I4rGKa1YlYaL5rl6rd9hGT9ywgjE
7QMcZiT2tSVRBSLCv8yompJVz/G+LrDyP7T8JrfgGyieKE50as24DXLOWXnWnHLB
EQxzTsmRAgMBAAECggEADv1Tb4JNz3QvyedwwA4yi/7jNwYtyvYm+mPz+xew1pop
86RusQUwA3/0o8tTxWfLWQzxq17Gyr3v9idG+HT89uHfd05FMhge4a1FXhtCgqtZ
mzEGdW27FjOrEa/6jIiqWYBphtAembL3sOXsa711MwVHegTExloz+aU2kuPRqfyf
ATDXPYw1J/sLd3Xamr3Y0LJ/yiVR3M1WAkUX3myjbmss5O+yfK4HiY7xXCxF218C
YVb3r7caIgUruV/D1hQncIZR95c2duBoQDcXoOEvQ67NhbDMTeixXTbAaBYP7Ypq
f87Ljdl3+KuX9XTx+RRGUFn9uQOECYCF5gEtU+fGYQKBgQD/+nKqweDiSrjqwmT5
DC0h9WBVEce08fumtfTC+1dlwv9ZNa8ns+UZWg7EBeKDeaE7s+Ru11KOq23allNV
gFV3Kqw4XOdI6g3TDW3XgYwtQ77JPTOymaQIEb6pxLzRrfwWPccSKIitlP0ff7VU
0WuBJWxZ2g6bPOw+egigbay4+QKBgQDSIMdPFsDh/X3E3NRD6lrQCXm/YWOICN/q
rG3SuTDisGMBmtsaLzvr7PVBk9QUd9eggwE2udlJyv73DtT5ZZUsQ7M37SlWGfHT
8pyb4OQy1xGPmc9NkolXlIaRi33WzCXKMP3w5a9MIHz4m6X0WgZQO/4XnM+I0/hY
7vUvbsgTWQKBgQCzSGk5ebMVKzqaie6Ik+OkbiS7UEmsTPNxTu2QBtOunUWU6Mm1
qASknfPLjUeZx/2KQDOVAlB7RkwZlcHmF41EemnGzCLdabinAjfVgZF5PoKIlcn4
pC1DzZHZe8a3oQD3Xutnp2YbFUe34Q1Sy55dBKX/xH8IcUIRfA1At7AKmQKBgQDL
q6/kLfbJVa4pOa6ZIbfiO70BToFt4sQ/L+DHRm9m2ncsoA/NQok/NY/Hf2UqbbrY
PwLXK668gwE9MOgn0FmV7QzyoXLWnRE0Uc2QnZwy1xmTag9wbh+nfzQsMNvJblkW
sQQDEm4mSLs5MYza2sOR04SHGJxkUKlAcmW/Ew7lCQKBgERBap56h1tVY2m+8gyz
Euiyfuo1Nk6/jygP9jL2bpmbAM2YgUrio/0YNgVDIVpWdRtQ2DI2WwuD9M7o6Fed
CekE2y43oejWM38zbrMChkwnWoNYddrSE+LoQhHUntzDkdM0JG6M6rvIgzEBU1Ko
Avd7QBCxvqXU+7acaZ2xxaV4
-----END PRIVATE KEY-----"#;

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl Into<OsString>) -> Self {
        let previous = std::env::var_os(key);
        let value = value.into();
        // SAFETY: test-only environment mutation happens while holding the test env lock.
        unsafe {
            std::env::set_var(key, &value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: test-only environment mutation happens while holding the test env lock.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn sample_config() -> AppConfig {
    AppConfig {
        auth: RuntimeAuthConfig {
            session_backend: SessionBackendPreference::File,
            signing_key_backend: DpopKeyBackend::File,
            pkcs11: None,
        },
        trade_controls: None,
    }
}

fn sample_env(server: &Server) -> EnvConfig {
    EnvConfig {
        graphql_url: server.url(),
        auth: AuthConfig {
            issuer: server.url(),
            audience: "aud".to_string(),
            client_id: "client-id".to_string(),
        },
    }
}

fn ensure_runtime_dpop_key(config: &AppConfig) {
    crate::dpop::DpopKeyMaterial::load_or_create_for_options(
        &crate::channel::current_dpop_runtime_options(config),
    )
    .expect("create runtime dpop key");
}

fn current_runtime_dpop_thumbprint(config: &AppConfig) -> String {
    crate::dpop::DpopKeyMaterial::load_existing_for_options(
        &crate::channel::current_dpop_runtime_options(config),
    )
    .expect("load runtime dpop key")
    .jwk_thumbprint()
    .expect("runtime dpop thumbprint")
}

/// Common environment, storage, and signing setup. Callers hold the test-env
/// lock for this fixture's lifetime; mocks and race synchronization stay local.
struct RefreshTestEnvironment {
    _channel_guard: crate::channel::TestEnvConfigOverrideGuard,
    _config_dir_guard: EnvGuard,
    tempdir: tempfile::TempDir,
    config: AppConfig,
    dpop_options: DpopRuntimeOptions,
    dpop_thumbprint: String,
}

impl RefreshTestEnvironment {
    fn new(env_cfg: &EnvConfig) -> Self {
        let config = sample_config();
        let tempdir = tempdir().expect("tempdir");
        let config_dir_guard = EnvGuard::set("SC_CONFIG_DIR", tempdir.path().as_os_str());
        let channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
        ensure_runtime_dpop_key(&config);
        let dpop_options = crate::channel::current_dpop_runtime_options(&config);
        let dpop_thumbprint = current_runtime_dpop_thumbprint(&config);
        Self {
            _channel_guard: channel_guard,
            _config_dir_guard: config_dir_guard,
            tempdir,
            config,
            dpop_options,
            dpop_thumbprint,
        }
    }

    fn session_manager(&self) -> SessionManager<StorageBackend> {
        SessionManager::with_store(StorageBackend::File(
            FileStore::new(self.tempdir.path().to_path_buf()).expect("file store"),
        ))
    }
}

fn spawn_update_after_refresh_start(
    updater_path: std::path::PathBuf,
    updater_session: StoredSession,
) -> (
    std::sync::mpsc::Sender<()>,
    std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
    std::thread::JoinHandle<()>,
) {
    let (refresh_started_tx, refresh_started_rx) = std::sync::mpsc::channel();
    let (updater_done_tx, updater_done_rx) = std::sync::mpsc::channel();
    let updater = std::thread::spawn(move || {
        refresh_started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("wait for refresh request to start");
        let mut other_manager = SessionManager::with_store(StorageBackend::File(
            FileStore::new(updater_path).expect("file store"),
        ));
        other_manager
            .save_active_locked(&updater_session)
            .expect("save newer session");
        updater_done_tx.send(()).expect("notify updater done");
    });
    (
        refresh_started_tx,
        std::sync::Arc::new(std::sync::Mutex::new(updater_done_rx)),
        updater,
    )
}

fn spawn_delete_after_refresh_start(
    updater_path: std::path::PathBuf,
) -> (
    std::sync::mpsc::Sender<()>,
    std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
    std::thread::JoinHandle<()>,
) {
    let (refresh_started_tx, refresh_started_rx) = std::sync::mpsc::channel();
    let (updater_done_tx, updater_done_rx) = std::sync::mpsc::channel();
    let updater = std::thread::spawn(move || {
        refresh_started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("wait for refresh request to start");
        let mut other_manager = SessionManager::with_store(StorageBackend::File(
            FileStore::new(updater_path).expect("file store"),
        ));
        other_manager
            .delete_active_locked()
            .expect("delete active session");
        updater_done_tx.send(()).expect("notify updater done");
    });
    (
        refresh_started_tx,
        std::sync::Arc::new(std::sync::Mutex::new(updater_done_rx)),
        updater,
    )
}

fn mock_oidc(server: &mut Server, issuer: &str, expected_calls: usize) -> (Mock, Mock) {
    let discovery = server
        .mock("GET", "/.well-known/openid-configuration")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "issuer": issuer,
                "jwks_uri": format!("{}/jwks", server.url())
            })
            .to_string(),
        )
        .expect(expected_calls)
        .create();

    let jwks = server
        .mock("GET", "/jwks")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "keys": [{
                    "kid": TEST_KID,
                    "kty": "RSA",
                    "use": "sig",
                    "alg": "RS256",
                    "n": TEST_RSA_N,
                    "e": TEST_RSA_E
                }]
            })
            .to_string(),
        )
        .expect(expected_calls)
        .create();

    (discovery, jwks)
}

fn make_verified_token(env_cfg: &EnvConfig, extra_claims: serde_json::Value) -> String {
    let mut claims = json!({
        "iss": env_cfg.auth.issuer,
        "aud": env_cfg.auth.audience,
        "exp": 9_999_999_999_i64,
        "person_id": "person-1"
    });

    if let Some(extra) = extra_claims.as_object() {
        let base = claims.as_object_mut().expect("claims should be object");
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(TEST_KID.to_string());
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("test private key"),
    )
    .expect("token should encode")
}

fn stored_session(
    access_token: String,
    refresh_token: Option<&str>,
    dpop_jwk_thumbprint: String,
    refresh_recovery_pending: bool,
) -> StoredSession {
    StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token,
            refresh_token: refresh_token.map(ToString::to_string),
            id_token: None,
            expires_at: Some(if refresh_recovery_pending {
                9_999_999_999
            } else {
                0
            }),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(dpop_jwk_thumbprint),
        mode: None,
        refresh_recovery_pending,
    }
}

struct FailOnWriteStore {
    value: std::sync::Mutex<Option<String>>,
    writes: std::sync::Mutex<usize>,
    fail_on_write: usize,
    failure: fn() -> anyhow::Error,
}

impl FailOnWriteStore {
    fn new(fail_on_write: usize) -> Self {
        Self {
            value: std::sync::Mutex::new(None),
            writes: std::sync::Mutex::new(0),
            fail_on_write,
            failure: || anyhow::anyhow!("injected session-store write failure"),
        }
    }
}

impl SecretStore for FailOnWriteStore {
    fn get(&self, _: &str) -> Result<Option<String>> {
        Ok(self.value.lock().expect("lock store").clone())
    }

    fn set(&self, _: &str, value: &str) -> Result<()> {
        let mut writes = self.writes.lock().expect("lock write count");
        *writes += 1;
        if *writes == self.fail_on_write {
            return Err((self.failure)());
        }
        *self.value.lock().expect("lock store") = Some(value.to_string());
        Ok(())
    }

    fn delete(&self, _: &str) -> Result<()> {
        *self.value.lock().expect("lock store") = None;
        Ok(())
    }
}

#[test]
fn unauthorized_detection_accepts_http_401_graphql_errors() {
    let err = anyhow::anyhow!("GraphQL HTTP error 401: unauthorized");
    assert!(is_unauthorized_graphql_error(&err));
}

#[test]
fn unauthorized_detection_accepts_graphql_unauthenticated_payload_errors() {
    let err = anyhow::Error::msg("GraphQL returned errors for WhoAmI (code: UNAUTHENTICATED)");
    assert!(is_unauthorized_graphql_error(&err));
}

#[test]
fn unauthorized_detection_ignores_non_auth_graphql_errors() {
    let err = anyhow::Error::msg("GraphQL returned errors for WhoAmI (code: FORBIDDEN)");
    assert!(!is_unauthorized_graphql_error(&err));
}

#[test]
fn unauthorized_detection_ignores_rate_limited_errors() {
    let err = anyhow::Error::msg(
        "RATE_LIMITED: backend rate limit exceeded during BrokerOverview; retry after 30s",
    );
    assert!(!is_unauthorized_graphql_error(&err));
}

#[test]
fn execute_with_refresh_retry_does_not_retry_rate_limited_errors() {
    let tmp = tempdir().expect("tempdir");
    let env_cfg = crate::channel::current_env_config();
    let config = sample_config();
    let store = StorageBackend::File(FileStore::new(tmp.path().to_path_buf()).expect("file store"));
    let mut session_manager = SessionManager::with_store(store);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let mut session = Session {
        access_token: "access-token".to_string(),
        refresh_token: Some("refresh-token".to_string()),
        id_token: None,
        expires_at: Some(9_999_999_999),
        person_id: "person-1".to_string(),
        source: LoginSource::DeviceCode,
    };
    let mut attempts = 0;

    let result: Result<()> = execute_with_refresh_retry(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        &mut session,
        &dpop_options,
        |_| {
            attempts += 1;
            Err(anyhow::anyhow!(
                "RATE_LIMITED: backend rate limit exceeded during BrokerOverview; retry after 30s"
            ))
        },
    );
    let err = result.expect_err("rate-limited call should fail");

    assert_eq!(attempts, 1);
    assert!(err.to_string().contains("RATE_LIMITED:"));
    assert_eq!(session.access_token, "access-token");
    assert_eq!(session.refresh_token.as_deref(), Some("refresh-token"));
}

#[test]
fn save_active_session_preserves_session_mode() {
    let _lock = crate::lock_test_env();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().as_os_str());
    let config = sample_config();
    let store = StorageBackend::File(FileStore::new(tmp.path().to_path_buf()).expect("file store"));
    let mut session_manager = SessionManager::with_store(store);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let session = Session {
        access_token: "access-token".to_string(),
        refresh_token: Some("refresh-token".to_string()),
        id_token: None,
        expires_at: Some(9_999_999_999),
        person_id: "person-1".to_string(),
        source: LoginSource::DeviceCode,
    };
    let base_session = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: "old-access-token".to_string(),
            refresh_token: Some("old-refresh-token".to_string()),
            id_token: None,
            expires_at: Some(1_000),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some("thumbprint-1".to_string()),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&base_session)
        .expect("save base session");

    save_active_session(
        &mut session_manager,
        &base_session,
        StoredSession {
            env: crate::channel::current_env(),
            session,
            dpop_jwk_thumbprint: Some("thumbprint-1".to_string()),
            mode: Some(SessionMode::LocalReadOnly),
            refresh_recovery_pending: false,
        },
        &dpop_options,
    )
    .expect("save refreshed session");

    let stored = session_manager
        .load_active()
        .expect("load active")
        .expect("stored session");
    assert_eq!(stored.mode, Some(SessionMode::LocalReadOnly));
}

#[test]
fn refresh_cleanup_does_not_interpret_error_text_as_a_deletion_instruction() {
    let _lock = crate::lock_test_env();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().as_os_str());
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
    let current = stored_session(
        "last-verified-access-token".to_string(),
        Some("refresh-token-rotated"),
        "thumbprint-1".to_string(),
        true,
    );
    session_manager
        .save_active(&current)
        .expect("save recovery");

    let error = handle_refresh_failure(
        &mut session_manager,
        &current,
        RefreshFailure::Other(anyhow::anyhow!(
            "local failure mentioning {REFRESH_RELOGIN_REQUIRED_PREFIX}"
        )),
    );

    assert!(error.to_string().contains("local failure"));
    assert_eq!(
        session_manager.load_active().expect("load active"),
        Some(current)
    );
}

#[test]
fn refresh_relogin_cleanup_keeps_newer_concurrently_saved_session() {
    let _lock = crate::lock_test_env();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().as_os_str());
    let store = StorageBackend::File(FileStore::new(tmp.path().to_path_buf()).expect("file store"));
    let mut session_manager = SessionManager::with_store(store);
    let stale = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: "access-token-1".to_string(),
            refresh_token: Some("refresh-token-1".to_string()),
            id_token: None,
            expires_at: Some(1_000),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some("thumbprint-1".to_string()),
        mode: None,
        refresh_recovery_pending: false,
    };
    let rotated = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: "access-token-2".to_string(),
            refresh_token: Some("refresh-token-2".to_string()),
            id_token: None,
            expires_at: Some(2_000),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some("thumbprint-1".to_string()),
        mode: None,
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&stale)
        .expect("save stale session");
    session_manager
        .save_active(&rotated)
        .expect("save rotated session");

    let err = handle_refresh_failure(
        &mut session_manager,
        &stale,
        refresh_relogin_required("Token refresh requires a new login"),
    );

    assert!(err.to_string().contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("rotated session should remain"),
        rotated
    );
}

#[test]
fn refresh_relogin_cleanup_deletes_matching_stale_session() {
    let _lock = crate::lock_test_env();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().as_os_str());
    let store = StorageBackend::File(FileStore::new(tmp.path().to_path_buf()).expect("file store"));
    let mut session_manager = SessionManager::with_store(store);
    let stale = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: "access-token-1".to_string(),
            refresh_token: Some("refresh-token-1".to_string()),
            id_token: None,
            expires_at: Some(1_000),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some("thumbprint-1".to_string()),
        mode: None,
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&stale)
        .expect("save stale session");

    let err = handle_refresh_failure(
        &mut session_manager,
        &stale,
        refresh_relogin_required("Token refresh requires a new login"),
    );

    assert!(err.to_string().contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
}

#[test]
fn marked_recovery_without_a_usable_refresh_token_requires_login_and_cleans_up() {
    let _lock = crate::lock_test_env();
    let server = Server::new();
    let env_cfg = sample_env(&server);
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let recovery = stored_session(
        "last-verified-access-token".to_string(),
        Some("   "),
        fixture.dpop_thumbprint.clone(),
        true,
    );
    let mut session_manager = fixture.session_manager();
    session_manager
        .save_active(&recovery)
        .expect("save recovery session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        recovery,
        &fixture.dpop_options,
    )
    .expect_err("corrupt recovery state must not be used");

    assert!(format!("{err:#}").contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
    assert_eq!(
        crate::machine::classify_error(&err).code,
        "refresh_relogin_required"
    );
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
}

#[test]
fn marked_recovery_retries_issuer_material_unavailability_without_sending_a_refresh() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let discovery = server
        .mock("GET", "/.well-known/openid-configuration")
        .with_status(503)
        .with_body("issuer-outage-sentinel")
        // Each verification attempt retries discovery once. Run recovery twice
        // to prove the marker survives an unchanged outage.
        .expect(4)
        .create();
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let recovery = stored_session(
        make_verified_token(
            &env_cfg,
            json!({ "https://de.scalable.capital/session_id": "sid-current" }),
        ),
        Some("refresh-token-rotated"),
        fixture.dpop_thumbprint.clone(),
        true,
    );
    let mut session_manager = fixture.session_manager();
    session_manager
        .save_active(&recovery)
        .expect("save recovery session");

    for _ in 0..2 {
        let active = session_manager
            .load_active()
            .expect("load active")
            .expect("recovery remains active");
        let err = refresh_loaded_session_if_needed(
            &mut session_manager,
            crate::channel::current_env(),
            &env_cfg,
            active,
            &fixture.dpop_options,
        )
        .expect_err("issuer outage before a new exchange is retryable");

        assert_eq!(
            crate::machine::classify_error(&err).code,
            "refresh_retryable"
        );
        let persisted = session_manager
            .load_active()
            .expect("load persisted recovery")
            .expect("recovery remains active");
        assert!(persisted.refresh_recovery_pending);
        assert_eq!(
            persisted.session.refresh_token.as_deref(),
            Some("refresh-token-rotated")
        );
    }

    discovery.assert();
}

#[test]
fn normal_refresh_does_not_claim_durable_recovery_before_any_token_response() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let discovery = server
        .mock("GET", "/.well-known/openid-configuration")
        .with_status(503)
        .expect(2)
        .create();
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let current = stored_session(
        make_verified_token(&env_cfg, json!({})),
        Some("refresh-token-current"),
        fixture.dpop_thumbprint.clone(),
        false,
    );
    let mut session_manager = fixture.session_manager();
    session_manager
        .save_active(&current)
        .expect("save current session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        current.clone(),
        &fixture.dpop_options,
    )
    .expect_err("issuer outage before an exchange should surface normally");

    assert!(!format!("{err:#}").contains(REFRESH_RETRYABLE_PREFIX));
    assert_ne!(
        crate::machine::classify_error(&err).code,
        "refresh_retryable"
    );
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("current session remains"),
        current
    );
    discovery.assert();
}

#[test]
fn marked_recovery_with_a_locally_rejected_preserved_token_requires_login() {
    let _lock = crate::lock_test_env();
    let server = Server::new();
    let env_cfg = sample_env(&server);
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let recovery = stored_session(
        "not-a-jwt".to_string(),
        Some("refresh-token-rotated"),
        fixture.dpop_thumbprint.clone(),
        true,
    );
    let mut session_manager = fixture.session_manager();
    session_manager
        .save_active(&recovery)
        .expect("save recovery session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        recovery,
        &fixture.dpop_options,
    )
    .expect_err("locally rejected preserved token must not be retried");

    assert_eq!(
        crate::machine::classify_error(&err).code,
        "refresh_relogin_required"
    );
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
}

#[test]
fn marked_recovery_deletes_the_matching_session_after_an_ambiguous_exchange_failure() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
    let _refresh = server
        .mock("POST", "/oauth/token")
        .with_status(503)
        .with_body(r#"{"error":"server_error"}"#)
        .expect(1)
        .create();
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let recovery = stored_session(
        make_verified_token(&env_cfg, json!({})),
        Some("refresh-token-rotated"),
        fixture.dpop_thumbprint.clone(),
        true,
    );
    let mut session_manager = fixture.session_manager();
    session_manager
        .save_active(&recovery)
        .expect("save recovery session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        recovery,
        &fixture.dpop_options,
    )
    .expect_err("a request without a usable response is ambiguous");

    assert_eq!(
        crate::machine::classify_error(&err).code,
        "refresh_relogin_required"
    );
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
}

#[test]
fn marked_recovery_deletes_the_matching_session_after_a_malformed_token_response() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
    let _refresh = server
        .mock("POST", "/oauth/token")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("not-json")
        .expect(1)
        .create();
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let recovery = stored_session(
        make_verified_token(&env_cfg, json!({})),
        Some("refresh-token-rotated"),
        fixture.dpop_thumbprint.clone(),
        true,
    );
    let mut session_manager = fixture.session_manager();
    session_manager
        .save_active(&recovery)
        .expect("save recovery session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        recovery,
        &fixture.dpop_options,
    )
    .expect_err("malformed successful response is ambiguous after the request is sent");

    assert_eq!(
        crate::machine::classify_error(&err).code,
        "refresh_relogin_required"
    );
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
}

#[test]
fn marked_recovery_deletes_the_matching_session_after_a_transport_disconnect() {
    let _lock = crate::lock_test_env();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind token test server");
    let addr = listener.local_addr().expect("read server address");
    let issuer = format!("http://{addr}");
    let env_cfg = EnvConfig {
        graphql_url: issuer.clone(),
        auth: AuthConfig {
            issuer: issuer.clone(),
            audience: "aud".to_string(),
            client_id: "client-id".to_string(),
        },
    };
    let worker = std::thread::spawn(move || {
        let read_request = |stream: &mut std::net::TcpStream| {
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).expect("read request");
                assert!(read > 0, "request closed before headers arrived");
                request.extend_from_slice(&buffer[..read]);
            }
        };
        let write_json = |stream: &mut std::net::TcpStream, body: String| {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write JSON response");
        };

        let (mut discovery, _) = listener.accept().expect("accept discovery request");
        read_request(&mut discovery);
        write_json(
            &mut discovery,
            json!({
                "issuer": issuer,
                "jwks_uri": format!("{issuer}/jwks")
            })
            .to_string(),
        );

        let (mut jwks, _) = listener.accept().expect("accept JWKS request");
        read_request(&mut jwks);
        write_json(
            &mut jwks,
            json!({
                "keys": [{
                    "kid": TEST_KID,
                    "kty": "RSA",
                    "use": "sig",
                    "alg": "RS256",
                    "n": TEST_RSA_N,
                    "e": TEST_RSA_E
                }]
            })
            .to_string(),
        );

        let (mut token_request, _) = listener.accept().expect("accept token request");
        read_request(&mut token_request);
        // Closing the accepted socket without an HTTP response simulates a
        // disconnect after the issuer received the refresh request.
    });

    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let recovery = stored_session(
        make_verified_token(&env_cfg, json!({})),
        Some("refresh-token-rotated"),
        fixture.dpop_thumbprint.clone(),
        true,
    );
    let mut session_manager = fixture.session_manager();
    session_manager
        .save_active(&recovery)
        .expect("save recovery session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        recovery,
        &fixture.dpop_options,
    )
    .expect_err("disconnect after token request is ambiguous");
    worker.join().expect("join token test server");

    assert_eq!(
        crate::machine::classify_error(&err).code,
        "refresh_relogin_required"
    );
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
}

#[test]
fn verified_replacement_for_another_person_requires_login() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let _refresh = server
        .mock("POST", "/oauth/token")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "access_token": make_verified_token(&env_cfg, json!({ "person_id": "person-2" })),
                "refresh_token": "refresh-token-next",
                "expires_in": 3600
            })
            .to_string(),
        )
        .expect(1)
        .create();
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let current = stored_session(
        make_verified_token(&env_cfg, json!({})),
        Some("refresh-token-current"),
        fixture.dpop_thumbprint.clone(),
        false,
    );
    let mut session_manager = fixture.session_manager();
    session_manager
        .save_active(&current)
        .expect("save current session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        current,
        &fixture.dpop_options,
    )
    .expect_err("a verified token for another person must not replace this session");

    assert_eq!(
        crate::machine::classify_error(&err).code,
        "refresh_relogin_required"
    );
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
}

#[test]
fn recovery_write_failure_requires_login_and_deletes_the_matching_original_record() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (discovery, jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
    let refresh = server
        .mock("POST", "/oauth/token")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "access_token": make_verified_token(&env_cfg, json!({ "jti": "replacement" })),
                "refresh_token": "refresh-token-next",
                "expires_in": 3600
            })
            .to_string(),
        )
        .expect(1)
        .create();
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let current = stored_session(
        make_verified_token(&env_cfg, json!({})),
        Some("refresh-token-current"),
        fixture.dpop_thumbprint.clone(),
        false,
    );
    // The initial save succeeds; the recovery write fails before the new
    // access token is verified. Cleanup must match the original record.
    let mut session_manager = SessionManager::with_store(FailOnWriteStore::new(2));
    session_manager.save_active(&current).expect("save current");

    let error = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        current,
        &fixture.dpop_options,
    )
    .expect_err("a rotated token that could not be saved requires login");

    assert!(
        error
            .to_string()
            .contains("rotated refresh token could not be persisted locally")
    );
    assert_eq!(
        crate::machine::classify_error(&error).code,
        "refresh_relogin_required"
    );
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
    discovery.assert();
    jwks.assert();
    refresh.assert();
}

#[test]
fn typed_final_session_save_failure_preserves_the_rotated_recovery_record() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let refreshed_access_token = make_verified_token(&env_cfg, json!({}));
    let _refresh = server
        .mock("POST", "/oauth/token")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "access_token": refreshed_access_token,
                "refresh_token": "refresh-token-next",
                "expires_in": 3600
            })
            .to_string(),
        )
        .expect(1)
        .create();
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let current = stored_session(
        make_verified_token(&env_cfg, json!({})),
        Some("refresh-token-current"),
        fixture.dpop_thumbprint.clone(),
        false,
    );
    // Initial save is write one, recovery persistence is write two, and only
    // the verified final save is made to fail.
    let mut store = FailOnWriteStore::new(3);
    store.failure = || {
        anyhow::Error::new(
            crate::session::SessionStorageError::secret_store_unavailable(
                keyring_core::Error::NoDefaultStore,
            ),
        )
    };
    let mut session_manager = SessionManager::with_store(store);
    session_manager
        .save_active(&current)
        .expect("save current session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        current.clone(),
        &fixture.dpop_options,
    )
    .expect_err("final persistence failure should be retryable");

    assert!(
        err.downcast_ref::<crate::session::SessionStorageError>()
            .is_some()
    );
    let classified = crate::machine::classify_error(&err);
    assert_eq!(classified.code, "refresh_retryable");
    assert_eq!(classified.exit_code, 30);
    let recovery = session_manager
        .load_active()
        .expect("load persisted recovery")
        .expect("recovery must remain active");
    assert!(recovery.refresh_recovery_pending);
    assert_eq!(recovery.session.access_token, current.session.access_token);
    assert_eq!(recovery.session.expires_at, Some(0));
    assert_eq!(
        recovery.session.refresh_token.as_deref(),
        Some("refresh-token-next")
    );
}

#[test]
fn refresh_retry_reuses_the_persisted_rotated_refresh_token() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let discovery_body = json!({
        "issuer": env_cfg.auth.issuer,
        "jwks_uri": format!("{}/jwks", server.url())
    })
    .to_string();
    let discovery_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = std::sync::Arc::clone(&discovery_calls);
    let discovery = server
        .mock("GET", "/.well-known/openid-configuration")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |writer| {
            if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                writer.write_all(b"invalid-json")
            } else {
                writer.write_all(discovery_body.as_bytes())
            }
        })
        .expect(4)
        .create();
    let jwks = server
        .mock("GET", "/jwks")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "keys": [{
                    "kid": TEST_KID,
                    "kty": "RSA",
                    "use": "sig",
                    "alg": "RS256",
                    "n": TEST_RSA_N,
                    "e": TEST_RSA_E
                }]
            })
            .to_string(),
        )
        .expect(3)
        .create();
    let refreshed_access_token = make_verified_token(&env_cfg, json!({}));
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "access_token": refreshed_access_token,
                "refresh_token": "refresh-token-next",
                "expires_in": 3600
            })
            .to_string(),
        )
        .create();
    let retried_access_token = make_verified_token(&env_cfg, json!({}));
    let _retry_refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-next".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "access_token": retried_access_token,
                "refresh_token": "refresh-token-final",
                "expires_in": 3600
            })
            .to_string(),
        )
        .create();

    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let stored_session = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: Some("id-token-current".to_string()),
            expires_at: Some(0),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: None,
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&stored_session)
        .expect("save active session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        stored_session.clone(),
        &fixture.dpop_options,
    )
    .expect_err("post-rotation verification should be retryable");

    assert_eq!(discovery_calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(format!("{err:#}").contains(REFRESH_RETRYABLE_PREFIX));
    assert_eq!(
        crate::machine::classify_error(&err).code,
        "refresh_retryable"
    );
    let mut recovery = session_manager
        .load_active()
        .expect("load active")
        .expect("recovery session should remain");
    assert_eq!(
        recovery.session.refresh_token.as_deref(),
        Some("refresh-token-next")
    );
    assert_eq!(
        recovery.session.access_token,
        stored_session.session.access_token
    );
    assert_eq!(recovery.session.id_token, stored_session.session.id_token);
    assert!(recovery.refresh_recovery_pending);
    assert_eq!(
        recovery.session.expires_at,
        Some(0),
        "recovery records remain refresh-due for marker-ignorant older binaries"
    );

    // The marker, rather than expiry, forces recovery. This also models a
    // record which has been written by a newer CLI and then inspected after
    // its deliberately refresh-due compatibility expiry.
    recovery.session.expires_at = Some(9_999_999_999);
    session_manager
        .save_active(&recovery)
        .expect("persist recovery session with future expiry");

    let recovered = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        recovery,
        &fixture.dpop_options,
    )
    .expect("retry should refresh with the preserved rotated refresh token");

    discovery.assert();
    jwks.assert();
    assert_eq!(discovery_calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    assert_eq!(recovered.access_token, retried_access_token);
    assert_eq!(
        recovered.refresh_token.as_deref(),
        Some("refresh-token-final")
    );
}

#[cfg(unix)]
#[test]
fn healthy_session_load_bypasses_a_held_refresh_lock() {
    let _lock = crate::lock_test_env();
    let server = Server::new();
    let env_cfg = sample_env(&server);
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let mut stored = stored_session(
        "healthy-access-token".to_string(),
        Some("refresh-token-current"),
        fixture.dpop_thumbprint.clone(),
        false,
    );
    stored.session.expires_at = Some(9_999_999_999);
    let mut session_manager = fixture.session_manager();
    session_manager.save_active(&stored).expect("save session");

    let held_lock = SessionRefreshLock::acquire().expect("hold refresh lock");
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let session_dir = fixture.tempdir.path().to_path_buf();
    let env = crate::channel::current_env();
    let worker_env_cfg = env_cfg.clone();
    let worker_options = fixture.dpop_options.clone();
    let worker_stored = stored.clone();
    let worker = std::thread::spawn(move || {
        let mut manager = SessionManager::with_store(StorageBackend::File(
            FileStore::new(session_dir).expect("file store"),
        ));
        result_tx
            .send(refresh_loaded_session_if_needed(
                &mut manager,
                env,
                &worker_env_cfg,
                worker_stored,
                &worker_options,
            ))
            .expect("send healthy load result");
    });

    let result = result_rx.recv_timeout(std::time::Duration::from_millis(100));
    drop(held_lock);
    worker.join().expect("join healthy-session worker");

    let loaded = result
        .expect("healthy session should not wait for the refresh lock")
        .expect("healthy session should load");
    assert_eq!(loaded, stored.session);
}

#[cfg(unix)]
#[test]
fn simultaneous_expiry_refreshes_are_serialized() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
    let refreshed_access_token = make_verified_token(&env_cfg, json!({}));
    let (first_refresh_started_tx, first_refresh_started_rx) = std::sync::mpsc::channel();
    let (complete_first_refresh_tx, complete_first_refresh_rx) = std::sync::mpsc::channel();
    let complete_first_refresh_rx =
        std::sync::Arc::new(std::sync::Mutex::new(complete_first_refresh_rx));
    let complete_first_refresh = std::sync::Arc::clone(&complete_first_refresh_rx);
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |writer| {
            first_refresh_started_tx
                .send(())
                .expect("notify first refresh request started");
            complete_first_refresh
                .lock()
                .expect("lock completion receiver")
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("allow first refresh response");
            writer.write_all(
                json!({
                    "access_token": refreshed_access_token,
                    "refresh_token": "refresh-token-next",
                    "expires_in": 3600
                })
                .to_string()
                .as_bytes(),
            )
        })
        .expect(1)
        .create();

    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let env = crate::channel::current_env();
    let stored_session = StoredSession {
        env,
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(0),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: None,
        refresh_recovery_pending: false,
    };
    let mut initial_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(fixture.tempdir.path().to_path_buf()).expect("file store"),
    ));
    initial_manager
        .save_active(&stored_session)
        .expect("save active session");

    let session_path = fixture.tempdir.path().to_path_buf();
    let first_env_cfg = env_cfg.clone();
    let first_dpop_options = fixture.dpop_options.clone();
    let first_stored_session = stored_session.clone();
    let first = std::thread::spawn(move || {
        let mut manager = SessionManager::with_store(StorageBackend::File(
            FileStore::new(session_path).expect("file store"),
        ));
        refresh_loaded_session_if_needed(
            &mut manager,
            env,
            &first_env_cfg,
            first_stored_session,
            &first_dpop_options,
        )
    });

    first_refresh_started_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("wait for first refresh request");

    let second_path = fixture.tempdir.path().to_path_buf();
    let second_env_cfg = env_cfg.clone();
    let second_dpop_options = fixture.dpop_options.clone();
    let second_stored_session = stored_session.clone();
    let (second_started_tx, second_started_rx) = std::sync::mpsc::channel();
    let (second_finished_tx, second_finished_rx) = std::sync::mpsc::channel();
    let second = std::thread::spawn(move || {
        second_started_tx
            .send(())
            .expect("notify second refresh start");
        let mut manager = SessionManager::with_store(StorageBackend::File(
            FileStore::new(second_path).expect("file store"),
        ));
        let result = refresh_loaded_session_if_needed(
            &mut manager,
            env,
            &second_env_cfg,
            second_stored_session,
            &second_dpop_options,
        );
        second_finished_tx
            .send(())
            .expect("notify second refresh completion");
        result
    });

    second_started_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("wait for second refresh attempt");
    assert!(matches!(
        second_finished_rx.recv_timeout(std::time::Duration::from_millis(100)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));

    complete_first_refresh_tx
        .send(())
        .expect("complete first refresh response");
    let first_session = first
        .join()
        .expect("join first refresh")
        .expect("first refresh should succeed");
    let second_session = second
        .join()
        .expect("join second refresh")
        .expect("second refresh should use the completed session");

    assert_eq!(first_session, second_session);
    assert_eq!(
        first_session.refresh_token.as_deref(),
        Some("refresh-token-next")
    );
}

#[test]
fn refresh_discards_recovery_session_when_the_replacement_access_token_is_rejected() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "access_token": "not-a-jwt",
                "refresh_token": "refresh-token-next",
                "expires_in": 3600
            })
            .to_string(),
        )
        .create();

    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let stored_session = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(0),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: None,
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&stored_session)
        .expect("save active session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        stored_session,
        &fixture.dpop_options,
    )
    .expect_err("malformed replacement access token should require relogin");

    assert!(format!("{err:#}").contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
    assert_eq!(
        crate::machine::classify_error(&err).code,
        "refresh_relogin_required"
    );
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
}

#[test]
fn refresh_loaded_session_if_needed_reloads_a_compatible_newer_session_under_lock() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
    let stale_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-stale" }),
    );
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-stale".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-stale".into()),
        ]))
        .with_status(400)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":"invalid_grant","error_description":"reuse detected"}"#)
        .create();

    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let stale = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: stale_access_token,
            refresh_token: Some("refresh-token-stale".to_string()),
            id_token: None,
            expires_at: Some(0),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint.clone()),
        mode: None,
        refresh_recovery_pending: false,
    };
    let rotated = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: "access-token-rotated".to_string(),
            refresh_token: Some("refresh-token-rotated".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: None,
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&rotated)
        .expect("save rotated session");

    let refreshed = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        stale,
        &fixture.dpop_options,
    )
    .expect("a compatible newer session should be used");

    assert_eq!(refreshed, rotated.session);
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("rotated session should remain"),
        rotated
    );
}

#[test]
fn refresh_loaded_session_if_needed_rejects_an_incompatible_session_replaced_before_lock() {
    let _lock = crate::lock_test_env();
    let server = Server::new();
    let env_cfg = sample_env(&server);
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let stale = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: "access-token-stale".to_string(),
            refresh_token: Some("refresh-token-stale".to_string()),
            id_token: None,
            expires_at: Some(0),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint.clone()),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    let replacement = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: "access-token-other".to_string(),
            refresh_token: Some("refresh-token-other".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-2".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&replacement)
        .expect("save replacement session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        stale,
        &fixture.dpop_options,
    )
    .expect_err("a different active identity must not be adopted");

    assert!(format!("{err:#}").contains("different identity or session mode"));
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("replacement session should remain"),
        replacement
    );
}

#[test]
fn refresh_loaded_session_if_needed_uses_authoritative_newer_saved_session_on_success_race() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let refreshed_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-refreshed" }),
    );
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let current = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(0),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint.clone()),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&current)
        .expect("save current session");

    let newer = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-newer" }),
            ),
            refresh_token: Some("refresh-token-newer".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(fixture.tempdir.path().to_path_buf(), newer.clone());
    let refresh_body_token = refreshed_access_token.clone();
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |w| {
            refresh_started_tx
                .send(())
                .expect("notify refresh request started");
            updater_done_rx
                .lock()
                .expect("lock updater receiver")
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("wait for newer session to be saved");
            w.write_all(
                json!({
                    "access_token": refresh_body_token,
                    "refresh_token": "refresh-token-next",
                    "expires_in": 3600
                })
                .to_string()
                .as_bytes(),
            )
        })
        .create();

    let refreshed = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        current,
        &fixture.dpop_options,
    )
    .expect("refresh should succeed");
    updater.join().expect("join updater");

    assert_eq!(refreshed, newer.session);
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("newer session should remain"),
        newer
    );
}

#[test]
fn execute_with_refresh_retry_rejects_a_changed_identity_before_refreshing() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let mut session_manager = fixture.session_manager();
    let mut replacement = stored_session(
        make_verified_token(&env_cfg, json!({})),
        Some("replacement-refresh"),
        fixture.dpop_thumbprint.clone(),
        false,
    );
    let mut in_memory_session = replacement.session.clone();
    replacement.session.person_id = "person-2".to_string();
    session_manager
        .save_active(&replacement)
        .expect("save replacement login");
    let refresh = server.mock("POST", "/oauth/token").expect(0).create();
    let mut attempts = 0;

    let result: Result<()> = execute_with_refresh_retry(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        &mut in_memory_session,
        &fixture.dpop_options,
        |_| {
            attempts += 1;
            Err(anyhow::anyhow!("GraphQL HTTP error 401: unauthorized"))
        },
    );

    let error = result.expect_err("changed identity must stop the retry");
    assert!(error.is::<SessionChanged>());
    assert_eq!(attempts, 1);
    assert_eq!(in_memory_session.person_id, "person-1");
    assert_eq!(
        session_manager
            .load_required_active()
            .expect("saved session"),
        replacement
    );
    refresh.assert();
}

#[test]
fn execute_with_refresh_retry_uses_stored_session_snapshot_and_preserves_mode() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let refreshed_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-refreshed" }),
    );
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "access_token": refreshed_access_token,
                "refresh_token": "refresh-token-next",
                "expires_in": 3600
            })
            .to_string(),
        )
        .create();

    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let current_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-current" }),
    );
    session_manager
        .save_active(&StoredSession {
            env: crate::channel::current_env(),
            session: Session {
                access_token: current_access_token,
                refresh_token: Some("refresh-token-current".to_string()),
                id_token: None,
                expires_at: Some(9_999_999_999),
                person_id: "person-1".to_string(),
                source: LoginSource::DeviceCode,
            },
            dpop_jwk_thumbprint: Some(runtime_thumbprint),
            mode: Some(SessionMode::LocalReadOnly),
            refresh_recovery_pending: false,
        })
        .expect("save current session");

    let mut in_memory_session = Session {
        access_token: make_verified_token(
            &env_cfg,
            json!({ "https://de.scalable.capital/session_id": "sid-stale" }),
        ),
        refresh_token: Some("refresh-token-stale".to_string()),
        id_token: None,
        expires_at: Some(9_999_999_999),
        person_id: "person-1".to_string(),
        source: LoginSource::DeviceCode,
    };
    let mut attempts = 0;

    let result = execute_with_refresh_retry(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        &mut in_memory_session,
        &fixture.dpop_options,
        |token| {
            attempts += 1;
            if attempts == 1 {
                return Err(anyhow::anyhow!("GraphQL HTTP error 401: unauthorized"));
            }
            assert_eq!(token, refreshed_access_token);
            Ok("ok")
        },
    )
    .expect("retry should succeed");

    assert_eq!(result, "ok");
    assert_eq!(attempts, 2);
    assert_eq!(
        in_memory_session.refresh_token.as_deref(),
        Some("refresh-token-next")
    );
    let stored = session_manager
        .load_active()
        .expect("load active")
        .expect("stored session");
    assert_eq!(stored.mode, Some(SessionMode::LocalReadOnly));
    assert_eq!(
        stored.session.refresh_token.as_deref(),
        Some("refresh-token-next")
    );
}

#[test]
fn execute_with_refresh_retry_keeps_newer_saved_session_on_relogin_failure() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let current = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&current)
        .expect("save current session");

    let newer = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-newer" }),
            ),
            refresh_token: Some("refresh-token-newer".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: current.dpop_jwk_thumbprint.clone(),
        mode: current.mode,
        refresh_recovery_pending: false,
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(fixture.tempdir.path().to_path_buf(), newer.clone());
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(400)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |w| {
            refresh_started_tx
                .send(())
                .expect("notify refresh request started");
            updater_done_rx
                .lock()
                .expect("lock updater receiver")
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("wait for newer session to be saved");
            w.write_all(br#"{"error":"invalid_grant","error_description":"reuse detected"}"#)
        })
        .create();

    let mut in_memory_session = Session {
        access_token: make_verified_token(
            &env_cfg,
            json!({ "https://de.scalable.capital/session_id": "sid-stale" }),
        ),
        refresh_token: Some("refresh-token-stale".to_string()),
        id_token: None,
        expires_at: Some(9_999_999_999),
        person_id: "person-1".to_string(),
        source: LoginSource::DeviceCode,
    };

    let result: Result<()> = execute_with_refresh_retry(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        &mut in_memory_session,
        &fixture.dpop_options,
        |_| Err(anyhow::anyhow!("GraphQL HTTP error 401: unauthorized")),
    );
    let err = result.expect_err("refresh should require relogin");
    updater.join().expect("join updater");

    assert!(format!("{err:#}").contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("newer session should remain"),
        newer
    );
}

#[test]
fn execute_with_refresh_retry_uses_authoritative_newer_saved_session_on_success_race() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let refreshed_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-refreshed" }),
    );
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let current = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint.clone()),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&current)
        .expect("save current session");

    let newer = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-newer" }),
            ),
            refresh_token: Some("refresh-token-newer".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(fixture.tempdir.path().to_path_buf(), newer.clone());
    let refresh_body_token = refreshed_access_token.clone();
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |w| {
            refresh_started_tx
                .send(())
                .expect("notify refresh request started");
            updater_done_rx
                .lock()
                .expect("lock updater receiver")
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("wait for newer session to be saved");
            w.write_all(
                json!({
                    "access_token": refresh_body_token,
                    "refresh_token": "refresh-token-next",
                    "expires_in": 3600
                })
                .to_string()
                .as_bytes(),
            )
        })
        .create();

    let mut in_memory_session = Session {
        access_token: make_verified_token(
            &env_cfg,
            json!({ "https://de.scalable.capital/session_id": "sid-stale" }),
        ),
        refresh_token: Some("refresh-token-stale".to_string()),
        id_token: None,
        expires_at: Some(9_999_999_999),
        person_id: "person-1".to_string(),
        source: LoginSource::DeviceCode,
    };
    let mut attempts = 0;

    let result = execute_with_refresh_retry(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        &mut in_memory_session,
        &fixture.dpop_options,
        |token| {
            attempts += 1;
            if attempts == 1 {
                return Err(anyhow::anyhow!("GraphQL HTTP error 401: unauthorized"));
            }
            assert_eq!(token, newer.session.access_token);
            Ok("ok")
        },
    )
    .expect("retry should succeed");
    updater.join().expect("join updater");

    assert_eq!(result, "ok");
    assert_eq!(attempts, 2);
    assert_eq!(in_memory_session, newer.session);
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("newer session should remain"),
        newer
    );
}

#[test]
fn execute_with_refresh_retry_fails_if_active_session_disappears_before_refresh_persist() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let refreshed_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-refreshed" }),
    );
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let current = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&current)
        .expect("save current session");

    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_delete_after_refresh_start(fixture.tempdir.path().to_path_buf());
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |w| {
            refresh_started_tx
                .send(())
                .expect("notify refresh request started");
            updater_done_rx
                .lock()
                .expect("lock updater receiver")
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("wait for active session to be deleted");
            w.write_all(
                json!({
                    "access_token": refreshed_access_token,
                    "refresh_token": "refresh-token-next",
                    "expires_in": 3600
                })
                .to_string()
                .as_bytes(),
            )
        })
        .create();

    let mut in_memory_session = Session {
        access_token: make_verified_token(
            &env_cfg,
            json!({ "https://de.scalable.capital/session_id": "sid-stale" }),
        ),
        refresh_token: Some("refresh-token-stale".to_string()),
        id_token: None,
        expires_at: Some(9_999_999_999),
        person_id: "person-1".to_string(),
        source: LoginSource::DeviceCode,
    };
    let mut attempts = 0;

    let result: Result<()> = execute_with_refresh_retry(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        &mut in_memory_session,
        &fixture.dpop_options,
        |_| {
            attempts += 1;
            Err(anyhow::anyhow!("GraphQL HTTP error 401: unauthorized"))
        },
    );
    let err = result.expect_err("retry should fail once active session disappears");
    updater.join().expect("join updater");

    assert_eq!(attempts, 1);
    assert!(format!("{err:#}").contains("another process changed the active session"));
    assert!(
        session_manager
            .load_active()
            .expect("load active")
            .is_none()
    );
    assert_eq!(
        in_memory_session.refresh_token.as_deref(),
        Some("refresh-token-stale")
    );
}

#[test]
fn execute_with_refresh_retry_fails_if_race_winner_changes_identity() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let refreshed_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-refreshed" }),
    );
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let current = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint.clone()),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&current)
        .expect("save current session");

    let newer = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-other" }),
            ),
            refresh_token: Some("refresh-token-other".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-2".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(fixture.tempdir.path().to_path_buf(), newer.clone());
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |w| {
            refresh_started_tx
                .send(())
                .expect("notify refresh request started");
            updater_done_rx
                .lock()
                .expect("lock updater receiver")
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("wait for competing session to be saved");
            w.write_all(
                json!({
                    "access_token": refreshed_access_token,
                    "refresh_token": "refresh-token-next",
                    "expires_in": 3600
                })
                .to_string()
                .as_bytes(),
            )
        })
        .create();

    let mut in_memory_session = Session {
        access_token: make_verified_token(
            &env_cfg,
            json!({ "https://de.scalable.capital/session_id": "sid-stale" }),
        ),
        refresh_token: Some("refresh-token-stale".to_string()),
        id_token: None,
        expires_at: Some(9_999_999_999),
        person_id: "person-1".to_string(),
        source: LoginSource::DeviceCode,
    };
    let mut attempts = 0;

    let result: Result<()> = execute_with_refresh_retry(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        &mut in_memory_session,
        &fixture.dpop_options,
        |_| {
            attempts += 1;
            Err(anyhow::anyhow!("GraphQL HTTP error 401: unauthorized"))
        },
    );
    let err = result.expect_err("retry should fail when the competing session changes identity");
    updater.join().expect("join updater");

    assert_eq!(attempts, 1);
    assert!(format!("{err:#}").contains("different identity or session mode"));
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("newer session should remain"),
        newer
    );
    assert_eq!(
        in_memory_session.refresh_token.as_deref(),
        Some("refresh-token-stale")
    );
}

#[test]
fn execute_with_refresh_retry_fails_if_race_winner_changes_mode_only() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let refreshed_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-refreshed" }),
    );
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let current = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint.clone()),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&current)
        .expect("save current session");

    let newer = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-mode" }),
            ),
            refresh_token: Some("refresh-token-mode".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: None,
        refresh_recovery_pending: false,
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(fixture.tempdir.path().to_path_buf(), newer.clone());
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |w| {
            refresh_started_tx
                .send(())
                .expect("notify refresh request started");
            updater_done_rx
                .lock()
                .expect("lock updater receiver")
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("wait for competing session to be saved");
            w.write_all(
                json!({
                    "access_token": refreshed_access_token,
                    "refresh_token": "refresh-token-next",
                    "expires_in": 3600
                })
                .to_string()
                .as_bytes(),
            )
        })
        .create();

    let mut in_memory_session = Session {
        access_token: make_verified_token(
            &env_cfg,
            json!({ "https://de.scalable.capital/session_id": "sid-stale" }),
        ),
        refresh_token: Some("refresh-token-stale".to_string()),
        id_token: None,
        expires_at: Some(9_999_999_999),
        person_id: "person-1".to_string(),
        source: LoginSource::DeviceCode,
    };
    let mut attempts = 0;

    let result: Result<()> = execute_with_refresh_retry(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        &mut in_memory_session,
        &fixture.dpop_options,
        |_| {
            attempts += 1;
            Err(anyhow::anyhow!("GraphQL HTTP error 401: unauthorized"))
        },
    );
    let err = result.expect_err("retry should fail when the competing session changes mode");
    updater.join().expect("join updater");

    assert_eq!(attempts, 1);
    assert!(format!("{err:#}").contains("different identity or session mode"));
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("newer session should remain"),
        newer
    );
    assert_eq!(
        in_memory_session.refresh_token.as_deref(),
        Some("refresh-token-stale")
    );
}

#[test]
fn execute_with_refresh_retry_fails_if_race_winner_has_unusable_dpop_binding() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let refreshed_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-refreshed" }),
    );
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let current = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(0),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&current)
        .expect("save current session");

    let newer = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-newer" }),
            ),
            refresh_token: Some("refresh-token-newer".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some("different-thumbprint".to_string()),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(fixture.tempdir.path().to_path_buf(), newer.clone());
    let refresh_body_token = refreshed_access_token.clone();
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |w| {
            refresh_started_tx
                .send(())
                .expect("notify refresh request started");
            updater_done_rx
                .lock()
                .expect("lock updater receiver")
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("wait for competing session to be saved");
            w.write_all(
                json!({
                    "access_token": refresh_body_token,
                    "refresh_token": "refresh-token-next",
                    "expires_in": 3600
                })
                .to_string()
                .as_bytes(),
            )
        })
        .create();

    let mut in_memory_session = Session {
        access_token: make_verified_token(
            &env_cfg,
            json!({ "https://de.scalable.capital/session_id": "sid-stale" }),
        ),
        refresh_token: Some("refresh-token-stale".to_string()),
        id_token: None,
        expires_at: Some(9_999_999_999),
        person_id: "person-1".to_string(),
        source: LoginSource::DeviceCode,
    };
    let mut attempts = 0;

    let result: Result<()> = execute_with_refresh_retry(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        &mut in_memory_session,
        &fixture.dpop_options,
        |_| {
            attempts += 1;
            Err(anyhow::anyhow!("GraphQL HTTP error 401: unauthorized"))
        },
    );
    let err =
        result.expect_err("retry should fail when the competing session uses an unusable DPoP key");
    updater.join().expect("join updater");

    assert_eq!(attempts, 1);
    assert!(format!("{err:#}").contains(crate::dpop::DPOP_SESSION_KEY_RELOGIN_MESSAGE));
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("newer session should remain"),
        newer
    );
    assert_eq!(
        in_memory_session.refresh_token.as_deref(),
        Some("refresh-token-stale")
    );
}

#[test]
fn refresh_loaded_session_if_needed_fails_if_race_winner_has_unusable_dpop_binding() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let refreshed_access_token = make_verified_token(
        &env_cfg,
        json!({ "https://de.scalable.capital/session_id": "sid-refreshed" }),
    );
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let runtime_thumbprint = fixture.dpop_thumbprint.clone();
    let mut session_manager = fixture.session_manager();
    let current = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-current" }),
            ),
            refresh_token: Some("refresh-token-current".to_string()),
            id_token: None,
            expires_at: Some(0),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some(runtime_thumbprint),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    session_manager
        .save_active(&current)
        .expect("save current session");

    let newer = StoredSession {
        env: crate::channel::current_env(),
        session: Session {
            access_token: make_verified_token(
                &env_cfg,
                json!({ "https://de.scalable.capital/session_id": "sid-newer" }),
            ),
            refresh_token: Some("refresh-token-newer".to_string()),
            id_token: None,
            expires_at: Some(9_999_999_999),
            person_id: "person-1".to_string(),
            source: LoginSource::DeviceCode,
        },
        dpop_jwk_thumbprint: Some("different-thumbprint".to_string()),
        mode: Some(SessionMode::LocalReadOnly),
        refresh_recovery_pending: false,
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(fixture.tempdir.path().to_path_buf(), newer.clone());
    let refresh_body_token = refreshed_access_token.clone();
    let _refresh = server
        .mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("client_id".into(), "client-id".into()),
            Matcher::UrlEncoded("refresh_token".into(), "refresh-token-current".into()),
            Matcher::UrlEncoded("session_id".into(), "sid-current".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_chunked_body(move |w| {
            refresh_started_tx
                .send(())
                .expect("notify refresh request started");
            updater_done_rx
                .lock()
                .expect("lock updater receiver")
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("wait for competing session to be saved");
            w.write_all(
                json!({
                    "access_token": refresh_body_token,
                    "refresh_token": "refresh-token-next",
                    "expires_in": 3600
                })
                .to_string()
                .as_bytes(),
            )
        })
        .create();

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        current,
        &fixture.dpop_options,
    )
    .expect_err("refresh should fail when the competing session uses an unusable DPoP key");
    updater.join().expect("join updater");

    assert!(format!("{err:#}").contains(crate::dpop::DPOP_SESSION_KEY_RELOGIN_MESSAGE));
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("newer session should remain"),
        newer
    );
}

#[test]
fn portfolio_chart_execution_retries_unauthorized_and_preserves_read_only_session() {
    let _lock = crate::lock_test_env();
    let mut server = Server::new();
    let env_cfg = sample_env(&server);
    let (discovery, jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
    let fixture = RefreshTestEnvironment::new(&env_cfg);
    let mut manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(fixture.tempdir.path().to_path_buf()).expect("file store"),
    ));
    let current_token = make_verified_token(
        &env_cfg,
        json!({"https://de.scalable.capital/session_id": "sid-current"}),
    );
    manager
        .save_active(&StoredSession {
            env: crate::channel::current_env(),
            session: Session {
                access_token: current_token.clone(),
                refresh_token: Some("refresh-token".to_string()),
                id_token: None,
                expires_at: Some(9_999_999_999),
                person_id: "person-1".to_string(),
                source: LoginSource::DeviceCode,
            },
            dpop_jwk_thumbprint: Some(fixture.dpop_thumbprint.clone()),
            mode: Some(SessionMode::LocalReadOnly),
            refresh_recovery_pending: false,
        })
        .expect("save session");
    let refreshed_token = make_verified_token(&env_cfg, json!({}));
    let refresh = server.mock("POST", "/oauth/token")
        .match_body(Matcher::AllOf(vec![Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()), Matcher::UrlEncoded("refresh_token".into(), "refresh-token".into())]))
        .with_header("content-type", "application/json").with_body(json!({"access_token": refreshed_token, "refresh_token": "refresh-token-next", "expires_in": 3600}).to_string()).expect(1).create();
    let unauthorized = server
        .mock("POST", "/")
        .match_header("authorization", format!("DPoP {current_token}").as_str())
        .match_body(Matcher::PartialJson(
            json!({"operation_name": "BrokerPortfolioChart"}),
        ))
        .with_status(401)
        .expect(1)
        .create();
    let chart = server.mock("POST", "/").match_header("authorization", format!("DPoP {refreshed_token}").as_str())
        .match_header("dpop", Matcher::Regex(".+".to_string()))
        .match_body(Matcher::PartialJson(json!({"operation_name": "BrokerPortfolioChart", "variables": {"accountId": "person-1", "portfolioId": "portfolio-1"}})))
        .with_header("content-type", "application/json").with_body(json!({"data": {"account": {"id": "person-1", "brokerPortfolio": {"id": "portfolio-1", "timeseries": [{
            "timeframe": "MAX", "dataPoints": [], "closingReferencePoint": {"valuation": "42.00", "absoluteReturn": "-2.00", "timestampUtc": {"time": "2026-10-09T10:00:00Z"}}
        }]}}}}).to_string()).expect(1).create();
    let result = crate::broker_query_execution::execute_broker_chart(
        crate::cli::BrokerChartArgs {
            isin: None,
            portfolio_id: Some("portfolio-1".to_string()),
            timeframe: crate::cli::BrokerChartTimeframe::Max,
            json: true,
        },
        &fixture.config,
        &mut manager,
    )
    .expect("refreshed chart");
    assert_eq!(
        result["result"]["closing_reference_point"]["valuation"],
        "42.00"
    );
    let saved = manager
        .load_active()
        .expect("session")
        .expect("saved session");
    assert_eq!(saved.mode, Some(SessionMode::LocalReadOnly));
    assert_eq!(
        saved.session.refresh_token.as_deref(),
        Some("refresh-token-next")
    );
    unauthorized.assert();
    refresh.assert();
    chart.assert();
    discovery.assert();
    jwks.assert();
}
