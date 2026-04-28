use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::{SigningKey, VerifyingKey};
use p256::elliptic_curve::rand_core::OsRng;
use serde::{Deserialize, Serialize};

use super::{DpopKeyMaterial, DpopPublicJwk, DpopSigner};
use crate::config::{config_dir_path, write_private_file_atomic};

const DPOP_KEY_FILE_NAME: &str = "auth-signing-key.json";

#[derive(Debug, Serialize, Deserialize)]
struct StoredDpopKey {
    kty: String,
    crv: String,
    d: String,
}

impl DpopKeyMaterial {
    pub fn load_or_create_at(path: &Path) -> Result<Self> {
        if path.exists() {
            return Self::load_from_file(path);
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed creating DPoP key dir {}", parent.display()))?;
        }

        let material = Self {
            signer: DpopSigner::Software(SigningKey::random(&mut OsRng)),
        };
        material.save_to_file(path)?;
        Ok(material)
    }

    pub fn load_existing_at(path: &Path) -> Result<Self> {
        if !path.exists() {
            bail!("DPoP key file {} does not exist", path.display());
        }
        Self::load_from_file(path)
    }

    pub fn from_private_scalar_bytes(bytes: [u8; 32]) -> Result<Self> {
        let field_bytes = p256::FieldBytes::from(bytes);
        let signing_key = SigningKey::from_bytes(&field_bytes)
            .map_err(|err| anyhow!("Invalid DPoP private key bytes: {err}"))?;
        Ok(Self {
            signer: DpopSigner::Software(signing_key),
        })
    }

    fn load_from_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed reading DPoP key file {}", path.display()))?;
        let stored: StoredDpopKey = serde_json::from_str(&raw)
            .with_context(|| format!("Invalid DPoP key file JSON at {}", path.display()))?;

        if stored.kty != "EC" || stored.crv != "P-256" {
            bail!(
                "Unsupported DPoP key format in {} (expected EC/P-256)",
                path.display()
            );
        }

        let d_bytes = decode_private_scalar(&stored.d)?;
        Self::from_private_scalar_bytes(d_bytes)
    }

    fn save_to_file(&self, path: &Path) -> Result<()> {
        #[cfg(target_os = "macos")]
        let DpopSigner::Software(signing_key) = &self.signer else {
            bail!("Secure Enclave DPoP key material cannot be exported to file")
        };
        #[cfg(target_os = "linux")]
        let DpopSigner::Software(signing_key) = &self.signer else {
            bail!("PKCS#11-backed DPoP key material cannot be exported to file")
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let DpopSigner::Software(signing_key) = &self.signer;

        let private_scalar = signing_key.to_bytes();
        let payload = StoredDpopKey {
            kty: "EC".to_string(),
            crv: "P-256".to_string(),
            d: URL_SAFE_NO_PAD.encode(private_scalar),
        };

        let serialized = serde_json::to_vec_pretty(&payload)?;
        write_private_file_atomic(path, &serialized)
            .with_context(|| format!("Failed writing DPoP key file {}", path.display()))?;
        Ok(())
    }
}

pub(super) fn software_public_jwk(signing_key: &SigningKey) -> Result<DpopPublicJwk> {
    let verifying_key = VerifyingKey::from(signing_key);
    let encoded = verifying_key.to_encoded_point(false);

    let x = encoded
        .x()
        .context("DPoP public key x coordinate is missing")?;
    let y = encoded
        .y()
        .context("DPoP public key y coordinate is missing")?;

    Ok(DpopPublicJwk {
        kty: "EC".to_string(),
        crv: "P-256".to_string(),
        x: URL_SAFE_NO_PAD.encode(x),
        y: URL_SAFE_NO_PAD.encode(y),
    })
}

pub(super) fn default_key_path() -> Result<PathBuf> {
    Ok(config_dir_path()?.join(DPOP_KEY_FILE_NAME))
}

fn decode_private_scalar(encoded: &str) -> Result<[u8; 32]> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|err| anyhow!("Invalid base64url private key scalar: {err}"))?;

    if decoded.len() != 32 {
        bail!(
            "Invalid DPoP private key scalar length {}; expected 32 bytes",
            decoded.len()
        );
    }

    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&decoded);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_or_create_roundtrip_uses_same_key() {
        let tmp = tempdir().expect("temp dir");
        let path = tmp.path().join("auth-signing-key.json");

        let first = DpopKeyMaterial::load_or_create_at(&path).expect("create key");
        let second = DpopKeyMaterial::load_or_create_at(&path).expect("load existing key");

        assert_eq!(
            first.jwk_thumbprint().expect("thumbprint"),
            second.jwk_thumbprint().expect("thumbprint")
        );
    }

    #[test]
    fn load_existing_fails_without_creating_key() {
        let tmp = tempdir().expect("temp dir");
        let path = tmp.path().join("auth-signing-key.json");

        let err = match DpopKeyMaterial::load_existing_at(&path) {
            Ok(_) => panic!("missing key should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("does not exist"));
        assert!(!path.exists());
    }
}
