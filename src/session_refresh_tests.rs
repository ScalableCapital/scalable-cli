use anyhow::Result;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use mockito::{Matcher, Mock, Server};
use serde_json::json;
use std::ffi::OsString;
use tempfile::tempdir;

use super::*;
use crate::config::{
    AppConfig, AuthConfig, DpopKeyBackend, EnvConfig, RuntimeAuthConfig, SessionBackendPreference,
};
use crate::session::{FileStore, LoginSource, StorageBackend};

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
    let tmp = tempdir().expect("tempdir");
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
    };
    session_manager
        .save_active(&base_session)
        .expect("save base session");

    save_active_session(
        &mut session_manager,
        crate::channel::current_env(),
        &session,
        &base_session,
        Some(SessionMode::LocalReadOnly),
        &dpop_options,
        "thumbprint-1",
    )
    .expect("save refreshed session");

    let stored = session_manager
        .load_active()
        .expect("load active")
        .expect("stored session");
    assert_eq!(stored.mode, Some(SessionMode::LocalReadOnly));
}

#[test]
fn refresh_relogin_cleanup_keeps_newer_concurrently_saved_session() {
    let tmp = tempdir().expect("tempdir");
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
    };
    session_manager
        .save_active(&stale)
        .expect("save stale session");
    session_manager
        .save_active(&rotated)
        .expect("save rotated session");

    let err = clear_active_session_on_refresh_relogin_failure(
        &mut session_manager,
        Some(&stale),
        anyhow::anyhow!("{REFRESH_RELOGIN_REQUIRED_PREFIX} Token refresh requires a new login."),
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
    let tmp = tempdir().expect("tempdir");
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
    };
    session_manager
        .save_active(&stale)
        .expect("save stale session");

    let err = clear_active_session_on_refresh_relogin_failure(
        &mut session_manager,
        Some(&stale),
        anyhow::anyhow!("{REFRESH_RELOGIN_REQUIRED_PREFIX} Token refresh requires a new login."),
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
fn refresh_loaded_session_if_needed_keeps_newer_saved_session_on_relogin_failure() {
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

    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
    };
    session_manager
        .save_active(&rotated)
        .expect("save rotated session");

    let err = refresh_loaded_session_if_needed(
        &mut session_manager,
        crate::channel::current_env(),
        &env_cfg,
        stale,
        &dpop_options,
    )
    .expect_err("refresh should require relogin");

    assert!(format!("{err:#}").contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
    assert_eq!(
        session_manager
            .load_active()
            .expect("load active")
            .expect("rotated session should remain"),
        rotated
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
    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(tmp.path().to_path_buf(), newer.clone());
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
        &dpop_options,
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

    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
        &dpop_options,
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
    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(tmp.path().to_path_buf(), newer.clone());
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
        &dpop_options,
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
    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(tmp.path().to_path_buf(), newer.clone());
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
        &dpop_options,
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
    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
    };
    session_manager
        .save_active(&current)
        .expect("save current session");

    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_delete_after_refresh_start(tmp.path().to_path_buf());
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
        &dpop_options,
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
    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(tmp.path().to_path_buf(), newer.clone());
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
        &dpop_options,
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
    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(tmp.path().to_path_buf(), newer.clone());
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
        &dpop_options,
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
    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(tmp.path().to_path_buf(), newer.clone());
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
        &dpop_options,
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
    let config = sample_config();
    let tmp = tempdir().expect("tempdir");
    let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
    let _channel_guard = crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone());
    ensure_runtime_dpop_key(&config);
    let dpop_options = crate::channel::current_dpop_runtime_options(&config);
    let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
    let mut session_manager = SessionManager::with_store(StorageBackend::File(
        FileStore::new(tmp.path().to_path_buf()).expect("file store"),
    ));
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
    };
    let (refresh_started_tx, updater_done_rx, updater) =
        spawn_update_after_refresh_start(tmp.path().to_path_buf(), newer.clone());
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
        &dpop_options,
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
