use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::config::EnvConfig;
use crate::transport_security::{
    build_blocking_client_https_only_with_timeout, validate_https_url,
};

const OIDC_DISCOVERY_PATH: &str = "/.well-known/openid-configuration";
const PINNED_ALGORITHM: Algorithm = Algorithm::RS256;
const PINNED_ALGORITHM_NAME: &str = "RS256";
const CLOCK_SKEW_SECONDS: i64 = 60;
const REQUEST_TIMEOUT_SECONDS: u64 = 2;
const TOTAL_NETWORK_BUDGET_SECONDS: u64 = 5;
const PERSON_ID_CLAIM: &str = "person_id";
const PERSON_ID_NS_CLAIM: &str = "https://de.scalable.capital/person_id";
const PERSON_ID_NS_CAMEL_CLAIM: &str = "https://de.scalable.capital/personId";
const SESSION_ID_CLAIM: &str = "session_id";
const SESSION_ID_NS_CLAIM: &str = "https://de.scalable.capital/session_id";

#[derive(Debug, Clone)]
pub struct VerifiedTokenClaims {
    pub person_id: String,
    pub expires_at: Option<i64>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum VerificationMode {
    Strict,
    AllowExpired,
}

#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    issuer: String,
    jwks_uri: String,
}

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwkSigningKey>,
}

#[derive(Debug, Deserialize)]
struct JwkSigningKey {
    #[serde(default)]
    kid: Option<String>,
    #[serde(default)]
    kty: Option<String>,
    #[serde(rename = "use")]
    #[serde(default)]
    use_: Option<String>,
    #[serde(default)]
    alg: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
}

pub fn verify_access_token_strict(token: &str, env_cfg: &EnvConfig) -> Result<VerifiedTokenClaims> {
    verify_access_token(token, env_cfg, VerificationMode::Strict)
}

pub fn verify_access_token_allow_expired(
    token: &str,
    env_cfg: &EnvConfig,
) -> Result<VerifiedTokenClaims> {
    verify_access_token(token, env_cfg, VerificationMode::AllowExpired)
}

fn verify_access_token(
    token: &str,
    env_cfg: &EnvConfig,
    mode: VerificationMode,
) -> Result<VerifiedTokenClaims> {
    let expected_issuer =
        normalize_issuer(validate_https_url(&env_cfg.auth.issuer, "auth.issuer")?.as_str())?;
    let header = decode_header(token).context("Token is not a valid JWT header")?;
    if header.alg != PINNED_ALGORITHM {
        bail!(
            "Unsupported JWT algorithm '{:?}'; expected '{}'",
            header.alg,
            PINNED_ALGORITHM_NAME
        );
    }
    let kid = header
        .kid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("Token header is missing required 'kid'")?;

    let deadline = Instant::now() + Duration::from_secs(TOTAL_NETWORK_BUDGET_SECONDS);
    let discovery = fetch_oidc_discovery(&expected_issuer, deadline)?;
    let discovered_issuer = normalize_issuer(&discovery.issuer)
        .context("OIDC discovery response contains invalid issuer")?;
    if discovered_issuer != expected_issuer {
        bail!(
            "OIDC discovery issuer mismatch: expected '{}', got '{}'",
            expected_issuer,
            discovered_issuer
        );
    }

    let jwks_url = validate_https_url(&discovery.jwks_uri, "OIDC discovery jwks_uri")?;
    let jwks = fetch_jwks(jwks_url.as_str(), deadline)?;
    let (n, e) = select_rsa_signing_key(&jwks, kid)?;
    let decoding_key = DecodingKey::from_rsa_components(n, e)
        .context("Failed constructing RSA decoding key from JWKS components")?;

    let mut validation = Validation::new(PINNED_ALGORITHM);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;

    let token_data = decode::<Value>(token, &decoding_key, &validation)
        .context("JWT signature verification failed")?;

    validate_claims(&token_data.claims, &expected_issuer, env_cfg, mode)?;
    build_verified_claims(&token_data.claims)
}

fn fetch_oidc_discovery(expected_issuer: &str, deadline: Instant) -> Result<OidcDiscovery> {
    let url = format!("{expected_issuer}{OIDC_DISCOVERY_PATH}");
    fetch_json_with_retry(&url, "OIDC discovery", deadline)
}

fn fetch_jwks(jwks_url: &str, deadline: Instant) -> Result<JwksResponse> {
    fetch_json_with_retry(jwks_url, "JWKS", deadline)
}

fn fetch_json_with_retry<T: DeserializeOwned>(
    url: &str,
    label: &str,
    deadline: Instant,
) -> Result<T> {
    let validated_url = validate_https_url(url, label)
        .with_context(|| format!("{label} URL is invalid: {url}"))?
        .to_string();
    let mut last_error = None::<String>;
    for attempt in 0..2 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("{label} verification network budget exceeded");
        }
        let timeout = remaining.min(Duration::from_secs(REQUEST_TIMEOUT_SECONDS));
        let client = build_blocking_client_https_only_with_timeout(timeout)
            .context("Failed building HTTP client for token verification")?;

        let response = match client.get(&validated_url).send() {
            Ok(resp) => resp,
            Err(err) => {
                let retryable = is_retryable_transport_error(&err);
                last_error = Some(format!(
                    "failed to fetch {label} from {validated_url}: {err}"
                ));
                if attempt == 0 && retryable {
                    continue;
                }
                bail!(
                    "{}",
                    last_error
                        .as_deref()
                        .unwrap_or("unknown token verification transport error")
                );
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            let message = format!(
                "{label} request returned HTTP {} at {validated_url}: {}",
                status.as_u16(),
                body.trim()
            );
            if attempt == 0 && status.is_server_error() {
                last_error = Some(message);
                continue;
            }
            bail!("{message}");
        }

        return response
            .json::<T>()
            .with_context(|| format!("Failed parsing {label} JSON from {validated_url}"));
    }

    bail!(
        "{}",
        last_error.unwrap_or_else(|| format!("Failed to fetch {label} from {validated_url}"))
    )
}

fn is_retryable_transport_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect()
}

fn select_rsa_signing_key<'a>(jwks: &'a JwksResponse, kid: &str) -> Result<(&'a str, &'a str)> {
    let key = jwks
        .keys
        .iter()
        .find(|entry| entry.kid.as_deref() == Some(kid))
        .with_context(|| format!("JWKS does not contain key for kid '{kid}'"))?;

    if key.kty.as_deref() != Some("RSA") {
        bail!("JWKS key for kid '{kid}' has unexpected kty; expected RSA");
    }
    if key.use_.as_deref().is_some_and(|value| value != "sig") {
        bail!("JWKS key for kid '{kid}' has unexpected use; expected 'sig'");
    }
    if key
        .alg
        .as_deref()
        .is_some_and(|value| value != PINNED_ALGORITHM_NAME)
    {
        bail!(
            "JWKS key for kid '{kid}' has unexpected alg; expected {}",
            PINNED_ALGORITHM_NAME
        );
    }

    let n = key
        .n
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("JWKS RSA key is missing modulus component 'n'")?;
    let e = key
        .e
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("JWKS RSA key is missing exponent component 'e'")?;

    Ok((n, e))
}

fn validate_claims(
    claims: &Value,
    expected_issuer: &str,
    env_cfg: &EnvConfig,
    mode: VerificationMode,
) -> Result<()> {
    let issuer = read_required_string_claim(claims, "iss")?;
    let issuer_normalized = normalize_issuer(issuer)?;
    if issuer_normalized != expected_issuer {
        bail!(
            "Token issuer mismatch: expected '{}', got '{}'",
            expected_issuer,
            issuer_normalized
        );
    }

    let audience_claim = claims
        .get("aud")
        .context("Token is missing required 'aud' claim")?;
    let (audience_matches, audience_count) =
        audience_contains(audience_claim, &env_cfg.auth.audience)?;
    if !audience_matches {
        bail!(
            "Token audience mismatch: expected to contain '{}'",
            env_cfg.auth.audience
        );
    }

    if audience_count > 1 {
        let azp = read_required_string_claim(claims, "azp")
            .context("Token has multiple audiences but missing required 'azp' claim")?;
        if azp != env_cfg.auth.client_id {
            bail!(
                "Token authorized party mismatch: expected '{}', got '{}'",
                env_cfg.auth.client_id,
                azp
            );
        }
    }

    let exp = read_required_i64_claim(claims, "exp")?;
    let now = now_epoch_seconds();
    if matches!(mode, VerificationMode::Strict) && now > exp + CLOCK_SKEW_SECONDS {
        bail!("Token is expired");
    }

    if let Some(nbf) = read_optional_i64_claim(claims, "nbf")?
        && now + CLOCK_SKEW_SECONDS < nbf
    {
        bail!("Token is not valid yet (nbf claim)");
    }

    if let Some(iat) = read_optional_i64_claim(claims, "iat")?
        && iat > now + CLOCK_SKEW_SECONDS
    {
        bail!("Token iat claim is in the future beyond allowed clock skew");
    }

    Ok(())
}

fn build_verified_claims(claims: &Value) -> Result<VerifiedTokenClaims> {
    let person_id = [
        PERSON_ID_NS_CLAIM,
        PERSON_ID_NS_CAMEL_CLAIM,
        PERSON_ID_CLAIM,
    ]
    .iter()
    .find_map(|key| read_optional_string_claim(claims, key))
    .context("Token does not contain required person_id claim")?;

    Ok(VerifiedTokenClaims {
        person_id,
        expires_at: claims.get("exp").and_then(Value::as_i64),
        session_id: [SESSION_ID_NS_CLAIM, SESSION_ID_CLAIM]
            .iter()
            .find_map(|key| read_optional_string_claim(claims, key)),
    })
}

fn read_required_string_claim<'a>(claims: &'a Value, name: &str) -> Result<&'a str> {
    claims
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("Token is missing required '{name}' claim"))
}

fn read_optional_string_claim(claims: &Value, name: &str) -> Option<String> {
    claims
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn read_required_i64_claim(claims: &Value, name: &str) -> Result<i64> {
    claims
        .get(name)
        .and_then(Value::as_i64)
        .with_context(|| format!("Token is missing required '{name}' claim"))
}

fn read_optional_i64_claim(claims: &Value, name: &str) -> Result<Option<i64>> {
    match claims.get(name) {
        Some(value) => value
            .as_i64()
            .map(Some)
            .with_context(|| format!("Token '{name}' claim must be an integer")),
        None => Ok(None),
    }
}

fn audience_contains(audience_claim: &Value, expected: &str) -> Result<(bool, usize)> {
    match audience_claim {
        Value::String(single) => Ok((single == expected, 1)),
        Value::Array(values) => {
            if values.is_empty() {
                bail!("Token 'aud' array claim is empty");
            }
            let audience_values = values
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()
                .context("Token 'aud' claim must contain only strings")?;
            Ok((audience_values.contains(&expected), audience_values.len()))
        }
        _ => bail!("Token 'aud' claim has unexpected type"),
    }
}

fn normalize_issuer(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("Issuer must not be empty");
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use base64::Engine;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use mockito::Server;
    use serde_json::json;

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

    fn sample_env(issuer: String) -> EnvConfig {
        EnvConfig {
            graphql_url: "https://example.invalid/graphql".to_string(),
            auth: AuthConfig {
                issuer,
                audience: "aud".to_string(),
                client_id: "client-id".to_string(),
            },
        }
    }

    fn signed_rs256_token(claims: Value, kid: Option<&str>) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = kid.map(ToString::to_string);
        encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("test private key"),
        )
        .expect("rs256 token")
    }

    fn signed_hs256_token(claims: Value) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(b"secret"),
        )
        .expect("hs256 token")
    }

    fn none_alg_token(claims: &Value) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
        format!("{header}.{payload}.sig")
    }

    fn mock_discovery_and_jwks(
        server: &mut Server,
        issuer_in_discovery: &str,
        jwks_body: Value,
    ) -> (mockito::Mock, mockito::Mock) {
        let discovery = server
            .mock("GET", OIDC_DISCOVERY_PATH)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "issuer": issuer_in_discovery,
                    "jwks_uri": format!("{}/jwks", server.url()),
                })
                .to_string(),
            )
            .create();
        let jwks = server
            .mock("GET", "/jwks")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(jwks_body.to_string())
            .create();
        (discovery, jwks)
    }

    fn standard_jwks() -> Value {
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
    }

    fn baseline_claims(issuer: &str) -> Value {
        json!({
            "iss": issuer,
            "aud": "aud",
            "exp": now_epoch_seconds() + 300,
            "person_id": "person-1"
        })
    }

    #[test]
    fn verifies_valid_token_and_extracts_claims() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": "aud",
            "exp": now_epoch_seconds() + 300,
            "https://de.scalable.capital/person_id": "person-ns",
            "https://de.scalable.capital/personId": "person-camel",
            "person_id": "person-plain",
            "https://de.scalable.capital/session_id": "session-ns",
            "session_id": "session-plain"
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let verified = verify_access_token_strict(&token, &env_cfg).expect("should verify");

        assert_eq!(verified.person_id, "person-ns");
        assert_eq!(verified.session_id.as_deref(), Some("session-ns"));
        assert!(verified.expires_at.is_some());
    }

    #[test]
    fn rejects_invalid_signature() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let mut claims = baseline_claims(&env_cfg.auth.issuer);
        claims["person_id"] = json!("person-1");
        let token = signed_hs256_token(claims);
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("Unsupported JWT algorithm"));
    }

    #[test]
    fn rejects_wrong_issuer() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = json!({
            "iss": "https://other-issuer.example",
            "aud": "aud",
            "exp": now_epoch_seconds() + 300,
            "person_id": "person-1"
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("issuer mismatch"));
    }

    #[test]
    fn rejects_wrong_audience() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": "other-aud",
            "exp": now_epoch_seconds() + 300,
            "person_id": "person-1"
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("audience mismatch"));
    }

    #[test]
    fn requires_azp_for_multi_audience() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": ["aud", "other"],
            "exp": now_epoch_seconds() + 300,
            "person_id": "person-1"
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("missing required 'azp'"));
    }

    #[test]
    fn rejects_wrong_azp_for_multi_audience() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": ["aud", "other"],
            "azp": "wrong-client",
            "exp": now_epoch_seconds() + 300,
            "person_id": "person-1"
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("authorized party mismatch"));
    }

    #[test]
    fn accepts_correct_azp_for_multi_audience() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": ["aud", "other"],
            "azp": env_cfg.auth.client_id,
            "exp": now_epoch_seconds() + 300,
            "person_id": "person-1"
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let verified = verify_access_token_strict(&token, &env_cfg).expect("should verify");
        assert_eq!(verified.person_id, "person-1");
    }

    #[test]
    fn rejects_unknown_kid() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = baseline_claims(&env_cfg.auth.issuer);
        let token = signed_rs256_token(claims, Some("missing"));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("does not contain key for kid"));
    }

    #[test]
    fn rejects_missing_kid() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = baseline_claims(&env_cfg.auth.issuer);
        let token = signed_rs256_token(claims, None);
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("missing required 'kid'"));
    }

    #[test]
    fn rejects_discovery_issuer_mismatch() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, "https://mismatch.example", standard_jwks());

        let claims = baseline_claims(&env_cfg.auth.issuer);
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("OIDC discovery issuer mismatch"));
    }

    #[test]
    fn rejects_non_https_jwks_uri() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let _discovery = server
            .mock("GET", OIDC_DISCOVERY_PATH)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "issuer": env_cfg.auth.issuer,
                    "jwks_uri": "http://example.invalid/jwks"
                })
                .to_string(),
            )
            .create();
        let _jwks = server
            .mock("GET", "/jwks")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(standard_jwks().to_string())
            .create();

        let claims = baseline_claims(&env_cfg.auth.issuer);
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("must use https"));
    }

    #[test]
    fn rejects_alg_none_token() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = baseline_claims(&env_cfg.auth.issuer);
        let token = none_alg_token(&claims);
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("valid JWT header"));
    }

    #[test]
    fn rejects_unexpected_jwk_metadata() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let jwks = json!({
            "keys": [{
                "kid": TEST_KID,
                "kty": "EC",
                "use": "enc",
                "alg": "HS256",
                "n": TEST_RSA_N,
                "e": TEST_RSA_E
            }]
        });
        let (_discovery, _jwks) = mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, jwks);

        let claims = baseline_claims(&env_cfg.auth.issuer);
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("unexpected kty"));
    }

    #[test]
    fn enforces_expiration_in_strict_mode_but_allows_in_allow_expired_mode() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());

        let claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": "aud",
            "exp": now_epoch_seconds() - 120,
            "person_id": "person-1"
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));

        let strict_err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(strict_err.to_string().contains("expired"));

        let verified = verify_access_token_allow_expired(&token, &env_cfg)
            .expect("allow_expired should accept expired token");
        assert_eq!(verified.person_id, "person-1");
    }

    #[test]
    fn retries_once_on_transient_discovery_failure() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let first = server
            .mock("GET", OIDC_DISCOVERY_PATH)
            .with_status(500)
            .expect(1)
            .create();
        let second = server
            .mock("GET", OIDC_DISCOVERY_PATH)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "issuer": env_cfg.auth.issuer,
                    "jwks_uri": format!("{}/jwks", server.url()),
                })
                .to_string(),
            )
            .expect(1)
            .create();
        let jwks = server
            .mock("GET", "/jwks")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(standard_jwks().to_string())
            .expect(1)
            .create();

        let claims = baseline_claims(&env_cfg.auth.issuer);
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let verified =
            verify_access_token_strict(&token, &env_cfg).expect("verification should succeed");
        assert_eq!(verified.person_id, "person-1");

        first.assert();
        second.assert();
        jwks.assert();
    }

    #[test]
    fn does_not_retry_on_discovery_4xx() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let discovery = server
            .mock("GET", OIDC_DISCOVERY_PATH)
            .with_status(404)
            .expect(1)
            .create();

        let claims = baseline_claims(&env_cfg.auth.issuer);
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("HTTP 404"));
        discovery.assert();
    }

    #[test]
    fn enforces_operation_network_budget() {
        let err = fetch_json_with_retry::<Value>(
            "https://example.invalid/oidc",
            "OIDC discovery",
            Instant::now(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("network budget exceeded"));
    }

    #[test]
    fn person_id_claim_precedence_is_deterministic() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());
        let claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": "aud",
            "exp": now_epoch_seconds() + 300,
            "person_id": "plain",
            "https://de.scalable.capital/personId": "camel",
            "https://de.scalable.capital/person_id": "snake"
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let verified = verify_access_token_strict(&token, &env_cfg).unwrap();
        assert_eq!(verified.person_id, "snake");
    }

    #[test]
    fn fails_when_person_id_claim_is_missing() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());
        let claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": "aud",
            "exp": now_epoch_seconds() + 300
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let err = verify_access_token_strict(&token, &env_cfg).unwrap_err();
        assert!(err.to_string().contains("person_id"));
    }

    #[test]
    fn session_id_claim_precedence_is_deterministic() {
        let mut server = Server::new();
        let env_cfg = sample_env(server.url());
        let (_discovery, _jwks) =
            mock_discovery_and_jwks(&mut server, &env_cfg.auth.issuer, standard_jwks());
        let claims = json!({
            "iss": env_cfg.auth.issuer,
            "aud": "aud",
            "exp": now_epoch_seconds() + 300,
            "person_id": "p-1",
            "session_id": "plain-session",
            "https://de.scalable.capital/session_id": "ns-session"
        });
        let token = signed_rs256_token(claims, Some(TEST_KID));
        let verified = verify_access_token_strict(&token, &env_cfg).unwrap();
        assert_eq!(verified.session_id.as_deref(), Some("ns-session"));
    }
}
