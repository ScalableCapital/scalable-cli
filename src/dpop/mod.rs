use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::config::{DpopKeyBackend, Pkcs11Config};

mod pkcs11;
mod secure_enclave;
mod software;

pub(crate) const DPOP_SESSION_KEY_RELOGIN_MESSAGE: &str =
    "The DPoP signing key for the current session is missing or changed; run 'sc login' again.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DpopRuntimeOptions {
    pub key_backend: DpopKeyBackend,
    pub pkcs11: Option<Pkcs11Config>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DpopPublicJwk {
    pub kty: String,
    pub crv: String,
    pub x: String,
    pub y: String,
}

pub struct DpopKeyMaterial {
    signer: DpopSigner,
}

enum DpopSigner {
    Software(SigningKey),
    #[cfg(target_os = "macos")]
    SecureEnclave(secure_enclave::SecureEnclaveKey),
    #[cfg(target_os = "linux")]
    Pkcs11(pkcs11::Pkcs11Key),
}

impl DpopKeyMaterial {
    pub fn load_or_create_for_options(options: &DpopRuntimeOptions) -> Result<Self> {
        match options.key_backend {
            DpopKeyBackend::File => Self::load_or_create_default(),
            DpopKeyBackend::SecureEnclave => {
                #[cfg(target_os = "macos")]
                {
                    let key = secure_enclave::SecureEnclaveKey::load_or_create().context(
                        "Failed to load or create Secure Enclave DPoP key (check macOS app packaging, keychain access, and signing policy)",
                    )?;
                    Ok(Self {
                        signer: DpopSigner::SecureEnclave(key),
                    })
                }
                #[cfg(not(target_os = "macos"))]
                {
                    bail!("DPoP key backend 'secure_enclave' is only supported on macOS")
                }
            }
            DpopKeyBackend::Pkcs11 => {
                #[cfg(target_os = "linux")]
                {
                    let config = options.pkcs11.as_ref().context(
                        "DPoP key backend 'pkcs11' requires [auth.pkcs11] with module_path and key_uri",
                    )?;
                    let key = pkcs11::Pkcs11Key::load(config).context(
                        "Failed to load PKCS#11 DPoP key material (check module_path, key_uri, token login, and public key object availability)",
                    )?;
                    Ok(Self {
                        signer: DpopSigner::Pkcs11(key),
                    })
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = options;
                    bail!("DPoP key backend 'pkcs11' is only supported on Linux")
                }
            }
        }
    }

    pub fn load_existing_for_options(options: &DpopRuntimeOptions) -> Result<Self> {
        match options.key_backend {
            DpopKeyBackend::File => Self::load_existing_default(),
            DpopKeyBackend::SecureEnclave => {
                #[cfg(target_os = "macos")]
                {
                    let key = secure_enclave::SecureEnclaveKey::load_existing().context(
                        "Failed to load existing Secure Enclave DPoP key (check keychain access and signing policy)",
                    )?;
                    Ok(Self {
                        signer: DpopSigner::SecureEnclave(key),
                    })
                }
                #[cfg(not(target_os = "macos"))]
                {
                    bail!("DPoP key backend 'secure_enclave' is only supported on macOS")
                }
            }
            DpopKeyBackend::Pkcs11 => {
                #[cfg(target_os = "linux")]
                {
                    let config = options.pkcs11.as_ref().context(
                        "DPoP key backend 'pkcs11' requires [auth.pkcs11] with module_path and key_uri",
                    )?;
                    let key = pkcs11::Pkcs11Key::load(config).context(
                        "Failed to load PKCS#11 DPoP key material (check module_path, key_uri, token login, and public key object availability)",
                    )?;
                    Ok(Self {
                        signer: DpopSigner::Pkcs11(key),
                    })
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = options;
                    bail!("DPoP key backend 'pkcs11' is only supported on Linux")
                }
            }
        }
    }

    pub fn load_or_create_default() -> Result<Self> {
        let path = software::default_key_path()?;
        Self::load_or_create_at(&path)
    }

    pub fn load_existing_default() -> Result<Self> {
        let path = software::default_key_path()?;
        Self::load_existing_at(&path)
    }

    pub fn public_jwk(&self) -> Result<DpopPublicJwk> {
        match &self.signer {
            DpopSigner::Software(signing_key) => software::software_public_jwk(signing_key),
            #[cfg(target_os = "macos")]
            DpopSigner::SecureEnclave(key) => key.public_jwk(),
            #[cfg(target_os = "linux")]
            DpopSigner::Pkcs11(key) => key.public_jwk(),
        }
    }

    pub fn jwk_thumbprint(&self) -> Result<String> {
        let jwk = self.public_jwk()?;
        Ok(jwk_thumbprint(&jwk))
    }

    pub fn proof_for_request(
        &self,
        method: &str,
        target_url: &str,
        nonce: Option<&str>,
        access_token: Option<&str>,
    ) -> Result<String> {
        let iat = current_epoch_seconds();
        let jti = random_jti();
        self.proof_for_request_with_overrides(method, target_url, nonce, access_token, iat, &jti)
    }

    pub fn proof_for_request_with_overrides(
        &self,
        method: &str,
        target_url: &str,
        nonce: Option<&str>,
        access_token: Option<&str>,
        iat: i64,
        jti: &str,
    ) -> Result<String> {
        let htm = method.trim().to_uppercase();
        if htm.is_empty() {
            bail!("DPoP proof input invalid: HTTP method must be non-empty");
        }

        let htu = canonicalize_htu(target_url)?;
        let jti = jti.trim();
        if jti.is_empty() {
            bail!("DPoP proof input invalid: jti must be non-empty");
        }

        let mut claims = serde_json::Map::new();
        claims.insert("htm".to_string(), Value::String(htm));
        claims.insert("htu".to_string(), Value::String(htu));
        claims.insert("iat".to_string(), Value::from(iat));
        claims.insert("jti".to_string(), Value::String(jti.to_string()));

        if let Some(raw_nonce) = nonce.map(str::trim).filter(|value| !value.is_empty()) {
            claims.insert("nonce".to_string(), Value::String(raw_nonce.to_string()));
        }

        if let Some(token) = access_token
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            claims.insert("ath".to_string(), Value::String(access_token_hash(token)));
        }

        let header = json!({
            "typ": "dpop+jwt",
            "alg": "ES256",
            "jwk": self.public_jwk()?,
        });

        let encoded_header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header)?);
        let encoded_payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?);
        let signing_input = format!("{encoded_header}.{encoded_payload}");

        let signature = self.sign_proof_input(signing_input.as_bytes())?;
        let encoded_signature = URL_SAFE_NO_PAD.encode(signature);

        Ok(format!("{signing_input}.{encoded_signature}"))
    }

    fn sign_proof_input(&self, signing_input: &[u8]) -> Result<[u8; 64]> {
        match &self.signer {
            DpopSigner::Software(signing_key) => {
                let signature: Signature = signing_key.sign(signing_input);
                Ok(signature.to_bytes().into())
            }
            #[cfg(target_os = "macos")]
            DpopSigner::SecureEnclave(key) => key.sign_dpop_input(signing_input),
            #[cfg(target_os = "linux")]
            DpopSigner::Pkcs11(key) => key.sign_dpop_input(signing_input),
        }
    }
}

fn der_signature_to_raw_ecdsa(der_signature: &[u8]) -> Result<[u8; 64]> {
    let signature = Signature::from_der(der_signature)
        .map_err(|err| anyhow!("Invalid DER ECDSA signature from signer: {err}"))?;
    Ok(signature.to_bytes().into())
}

pub fn canonicalize_htu(target_url: &str) -> Result<String> {
    let mut parsed = Url::parse(target_url)
        .with_context(|| format!("Invalid DPoP target URL '{target_url}'"))?;

    parsed.set_query(None);
    parsed.set_fragment(None);

    Ok(parsed.to_string())
}

pub fn access_token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

pub fn jwk_thumbprint(jwk: &DpopPublicJwk) -> String {
    // RFC 7638 canonical member order for EC JWK thumbprints.
    let canonical = format!(
        "{{\"crv\":\"{}\",\"kty\":\"{}\",\"x\":\"{}\",\"y\":\"{}\"}}",
        jwk.crv, jwk.kty, jwk.x, jwk.y
    );

    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn random_jti() -> String {
    let mut bytes = [0_u8; 16];
    let mut rng = rand::rng();
    rng.fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn current_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::elliptic_curve::rand_core::OsRng;

    #[test]
    fn canonicalize_htu_strips_query_and_fragment() {
        let htu = canonicalize_htu("https://de.scalable.capital/api/graphql?foo=bar#frag")
            .expect("canonicalize");
        assert_eq!(htu, "https://de.scalable.capital/api/graphql");
    }

    #[test]
    fn proof_contains_required_claims_and_ath() {
        let key = DpopKeyMaterial::from_private_scalar_bytes([7_u8; 32]).expect("fixed key");

        let jwt = key
            .proof_for_request_with_overrides(
                "post",
                "https://de.scalable.capital/api/graphql?foo=bar",
                Some("nonce-1"),
                Some("access-token-1"),
                1_700_000_000,
                "jti-1",
            )
            .expect("proof");

        let parts = jwt.split('.').collect::<Vec<_>>();
        assert_eq!(parts.len(), 3);

        let header_json = URL_SAFE_NO_PAD.decode(parts[0]).expect("decode header");
        let payload_json = URL_SAFE_NO_PAD.decode(parts[1]).expect("decode payload");

        let header: Value = serde_json::from_slice(&header_json).expect("header json");
        let payload: Value = serde_json::from_slice(&payload_json).expect("payload json");

        assert_eq!(header["typ"], "dpop+jwt");
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["jwk"]["kty"], "EC");
        assert_eq!(header["jwk"]["crv"], "P-256");

        assert_eq!(payload["htm"], "POST");
        assert_eq!(payload["htu"], "https://de.scalable.capital/api/graphql");
        assert_eq!(payload["iat"], 1_700_000_000);
        assert_eq!(payload["jti"], "jti-1");
        assert_eq!(payload["nonce"], "nonce-1");
        assert_eq!(payload["ath"], access_token_hash("access-token-1"));
    }

    #[test]
    fn jwk_thumbprint_is_stable_for_fixed_key() {
        let key = DpopKeyMaterial::from_private_scalar_bytes([1_u8; 32]).expect("fixed key");
        let thumbprint = key.jwk_thumbprint().expect("thumbprint");
        assert_eq!(thumbprint, "Nrqg3-M_Xwtx-1tbtc1J7Xul2DyeC0bUSy9u_5NSG6g");
    }

    #[test]
    fn der_signature_conversion_produces_raw_64_bytes() {
        let key = SigningKey::random(&mut OsRng);
        let sig: Signature = key.sign(b"hello");
        let der = sig.to_der();

        let raw = der_signature_to_raw_ecdsa(der.as_bytes()).expect("der to raw");
        assert_eq!(raw.len(), 64);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn pkcs11_backend_fails_clearly_outside_linux() {
        let options = DpopRuntimeOptions {
            key_backend: DpopKeyBackend::Pkcs11,
            pkcs11: Some(Pkcs11Config {
                module_path: "/usr/lib/opensc-pkcs11.so".to_string(),
                key_uri: "pkcs11:token=YubiKey%20PIV;id=%01".to_string(),
            }),
        };

        let err = match DpopKeyMaterial::load_or_create_for_options(&options) {
            Ok(_) => panic!("pkcs11 should be unsupported outside linux"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("only supported on Linux"));
    }
}
