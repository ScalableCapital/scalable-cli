use anyhow::Result;
use serde_json::{Value, json};

use crate::cli::WhoamiArgs;
use crate::config::AppConfig;
use crate::graphql::run_whoami_query;
use crate::session::SessionManager;
use crate::transport_security::validate_env_transport_security;
use crate::{execute_with_refresh_retry, print_whoami_text, refresh_loaded_session_if_needed};

fn load_whoami_result(config: &AppConfig, session_manager: &mut SessionManager) -> Result<Value> {
    let dpop_options = crate::channel::current_dpop_runtime_options(config);
    let dpop_options = &dpop_options;
    let stored = session_manager.load_required_active()?;
    crate::channel::require_current_channel(stored.env)?;
    let env = stored.env;
    let env_cfg = crate::channel::current_env_config();
    validate_env_transport_security(&env_cfg)?;
    let mut session =
        refresh_loaded_session_if_needed(session_manager, env, &env_cfg, stored, dpop_options)?;

    let person_id = session.person_id.clone();
    execute_with_refresh_retry(
        session_manager,
        env,
        &env_cfg,
        &mut session,
        dpop_options,
        |token| run_whoami_query(&env_cfg.graphql_url, token, &person_id, dpop_options),
    )
}

pub(crate) fn run_human_whoami_command(
    args: WhoamiArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<()> {
    let result = load_whoami_result(config, session_manager)?;
    if args.json {
        let payload = json!({
            "personOverview": result
                .get("personOverview")
                .cloned()
                .unwrap_or(result.clone()),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_whoami_text(&result)?;
    }

    Ok(())
}

pub(crate) fn run_machine_whoami_command(
    _args: WhoamiArgs,
    config: &AppConfig,
    session_manager: &mut SessionManager,
) -> Result<Value> {
    let result = load_whoami_result(config, session_manager)?;
    Ok(json!({ "result": result }))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use mockito::{Matcher, Mock, Server};
    use serde_json::{Value, json};

    use super::*;
    use crate::auth::REFRESH_RELOGIN_REQUIRED_PREFIX;
    use crate::config::{
        AuthConfig, DpopKeyBackend, EnvConfig, RuntimeAuthConfig, SessionBackendPreference,
    };
    use crate::machine::classify_error;
    use crate::session::{
        FileStore, LoginSource, Session, SessionManager, StorageBackend, StoredSession,
    };

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

    fn file_session_manager(tempdir: &tempfile::TempDir) -> SessionManager {
        SessionManager::with_store(StorageBackend::File(
            FileStore::new(tempdir.path().to_path_buf()).expect("file store"),
        ))
    }

    struct TestChannelGuard {
        _override: crate::channel::TestEnvConfigOverrideGuard,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestChannelGuard {
        fn from_env(env_cfg: &EnvConfig) -> Self {
            let lock = crate::lock_test_env();
            Self {
                _lock: lock,
                _override: crate::channel::TestEnvConfigOverrideGuard::set(env_cfg.clone()),
            }
        }
    }

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

    fn make_verified_token(env_cfg: &EnvConfig, extra_claims: Value) -> String {
        let mut claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": env_cfg.auth.audience,
            "exp": 9_999_999_999_i64,
            "person_id": "p-1"
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

    fn sample_session(env_cfg: &EnvConfig, expires_at: i64) -> Session {
        Session {
            access_token: make_verified_token(env_cfg, json!({})),
            refresh_token: Some("refresh-1".to_string()),
            id_token: Some("id-token".to_string()),
            expires_at: Some(expires_at),
            person_id: "p-1".to_string(),
            source: LoginSource::DeviceCode,
        }
    }

    #[test]
    fn machine_whoami_preserves_refresh_relogin_required_classification() {
        let mut server = Server::new();
        let env_cfg = sample_env(&server);
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
        let _refresh = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("refresh_token".into(), "refresh-1".into()),
            ]))
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"invalid_grant","error_description":"reuse detected"}"#)
            .create();

        let _channel_guard = TestChannelGuard::from_env(&env_cfg);
        let config = sample_config();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        session_manager
            .save_active(&StoredSession {
                env: crate::channel::current_env(),
                session: sample_session(&env_cfg, 0),
                dpop_jwk_thumbprint: Some(current_runtime_dpop_thumbprint(&config)),
            })
            .expect("save active session");

        let err =
            run_machine_whoami_command(WhoamiArgs { json: false }, &config, &mut session_manager)
                .expect_err("refresh should require relogin");

        assert!(format!("{err:#}").contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
        assert!(!err.to_string().contains("No active session for dev"));
        assert_eq!(classify_error(&err).code, "refresh_relogin_required");
    }

    #[test]
    fn machine_whoami_clears_active_session_when_preflight_refresh_verification_fails() {
        let mut server = Server::new();
        let env_cfg = sample_env(&server);
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
        let _refresh = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("refresh_token".into(), "refresh-1".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"access_token":"not-a-jwt","refresh_token":"refresh-2","expires_in":3600}"#,
            )
            .create();

        let _channel_guard = TestChannelGuard::from_env(&env_cfg);
        let config = sample_config();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        session_manager
            .save_active(&StoredSession {
                env: crate::channel::current_env(),
                session: sample_session(&env_cfg, 0),
                dpop_jwk_thumbprint: Some(current_runtime_dpop_thumbprint(&config)),
            })
            .expect("save active session");

        let err =
            run_machine_whoami_command(WhoamiArgs { json: false }, &config, &mut session_manager)
                .expect_err("refresh should require relogin");

        assert!(format!("{err:#}").contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
        assert!(format!("{err:#}").contains("refreshed access token failed verification"));
        assert_eq!(classify_error(&err).code, "refresh_relogin_required");
        assert!(
            session_manager
                .load_active()
                .expect("load active")
                .is_none()
        );
    }

    #[test]
    fn machine_whoami_clears_active_session_when_retry_refresh_verification_fails() {
        let mut server = Server::new();
        let env_cfg = sample_env(&server);
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 1);
        let _whoami = server
            .mock("POST", "/")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"unauthorized"}"#)
            .create();
        let _refresh = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("refresh_token".into(), "refresh-1".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"access_token":"not-a-jwt","refresh_token":"refresh-2","expires_in":3600}"#,
            )
            .create();

        let _channel_guard = TestChannelGuard::from_env(&env_cfg);
        let config = sample_config();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        session_manager
            .save_active(&StoredSession {
                env: crate::channel::current_env(),
                session: sample_session(&env_cfg, 9_999_999_999),
                dpop_jwk_thumbprint: Some(current_runtime_dpop_thumbprint(&config)),
            })
            .expect("save active session");

        let err =
            run_machine_whoami_command(WhoamiArgs { json: false }, &config, &mut session_manager)
                .expect_err("refresh should require relogin");

        assert!(format!("{err:#}").contains(REFRESH_RELOGIN_REQUIRED_PREFIX));
        assert!(format!("{err:#}").contains("refreshed access token failed verification"));
        assert_eq!(classify_error(&err).code, "refresh_relogin_required");
        assert!(
            session_manager
                .load_active()
                .expect("load active")
                .is_none()
        );
    }

    #[test]
    fn machine_whoami_persists_rotated_refresh_token() {
        let mut server = Server::new();
        let env_cfg = sample_env(&server);
        let (_discovery, _jwks) = mock_oidc(&mut server, &env_cfg.auth.issuer, 2);
        let refreshed_access_token = make_verified_token(&env_cfg, json!({}));
        let _refresh = server
            .mock("POST", "/oauth/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("client_id".into(), "client-id".into()),
                Matcher::UrlEncoded("refresh_token".into(), "refresh-1".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "access_token": refreshed_access_token,
                    "refresh_token": "refresh-2",
                    "expires_in": 3600
                })
                .to_string(),
            )
            .create();
        let _whoami = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "data": {
                        "personOverview": {
                            "id": "p-1",
                            "locale": "de-DE",
                            "personalDetails": {
                                "firstName": "Ada",
                                "lastName": "Lovelace"
                            }
                        }
                    }
                })
                .to_string(),
            )
            .create();

        let _channel_guard = TestChannelGuard::from_env(&env_cfg);
        let config = sample_config();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        session_manager
            .save_active(&StoredSession {
                env: crate::channel::current_env(),
                session: sample_session(&env_cfg, 0),
                dpop_jwk_thumbprint: Some(current_runtime_dpop_thumbprint(&config)),
            })
            .expect("save active session");

        let result =
            run_machine_whoami_command(WhoamiArgs { json: false }, &config, &mut session_manager)
                .expect("whoami should succeed");

        assert_eq!(result["result"]["personOverview"]["id"], "p-1");
        let stored = session_manager
            .load_active()
            .expect("load active")
            .expect("stored session");
        assert_eq!(stored.session.refresh_token.as_deref(), Some("refresh-2"));
        let runtime_thumbprint = current_runtime_dpop_thumbprint(&config);
        assert_eq!(
            stored.dpop_jwk_thumbprint.as_deref(),
            Some(runtime_thumbprint.as_str())
        );
    }

    #[test]
    fn machine_whoami_requires_relogin_for_legacy_session_without_dpop_thumbprint() {
        let server = Server::new();
        let env_cfg = sample_env(&server);

        let _channel_guard = TestChannelGuard::from_env(&env_cfg);
        let config = sample_config();
        let tmp = tempfile::tempdir().expect("tempdir");
        let _cfg_guard = EnvGuard::set("SC_CONFIG_DIR", tmp.path().to_string_lossy().to_string());
        ensure_runtime_dpop_key(&config);
        let mut session_manager = file_session_manager(&tmp);
        session_manager
            .save_active(&StoredSession {
                env: crate::channel::current_env(),
                session: sample_session(&env_cfg, 9_999_999_999),
                dpop_jwk_thumbprint: None,
            })
            .expect("save active session");

        let err =
            run_machine_whoami_command(WhoamiArgs { json: false }, &config, &mut session_manager)
                .expect_err("legacy session should require relogin");

        assert_eq!(
            err.to_string(),
            crate::dpop::DPOP_SESSION_KEY_RELOGIN_MESSAGE
        );
        assert!(!err.to_string().contains("Stored active session is invalid"));
        assert_eq!(classify_error(&err).code, "refresh_relogin_required");
    }
}
