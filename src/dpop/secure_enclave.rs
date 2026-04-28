#[cfg(target_os = "macos")]
use anyhow::{Context, Result, anyhow, bail};
#[cfg(target_os = "macos")]
use base64::Engine;
#[cfg(target_os = "macos")]
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

#[cfg(target_os = "macos")]
use super::{DpopPublicJwk, der_signature_to_raw_ecdsa};

#[cfg(target_os = "macos")]
const DPOP_SECURE_ENCLAVE_LABEL: &str = "scalable.capital:scalable-cli:auth-signing-key:v2";
#[cfg(target_os = "macos")]
const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub(super) struct SecureEnclaveKey {
    private_key: security_framework::key::SecKey,
}

#[cfg(target_os = "macos")]
impl SecureEnclaveKey {
    pub(super) fn load_or_create() -> Result<Self> {
        if let Some(existing) = Self::find_existing()? {
            return Ok(existing);
        }

        Self::create_new()
    }

    pub(super) fn load_existing() -> Result<Self> {
        Self::find_existing()?.context("No existing Secure Enclave DPoP key found")
    }

    fn find_existing() -> Result<Option<Self>> {
        use security_framework::item::{
            ItemClass, ItemSearchOptions, KeyClass, Reference, SearchResult,
        };

        let mut search = ItemSearchOptions::new();
        search
            .class(ItemClass::key())
            .key_class(KeyClass::private())
            .label(DPOP_SECURE_ENCLAVE_LABEL)
            .load_refs(true)
            .limit(1);
        search.ignore_legacy_keychains();

        if let Ok(results) = search.search() {
            for result in results {
                if let SearchResult::Ref(Reference::Key(key)) = result {
                    return Ok(Some(Self { private_key: key }));
                }
            }
        }

        Ok(None)
    }

    fn create_new() -> Result<Self> {
        use core_foundation::error::CFError;
        use security_framework::access_control::{ProtectionMode, SecAccessControl};
        use security_framework::item::Location;
        use security_framework::key::{GenerateKeyOptions, KeyType, SecKey, Token};
        use security_framework::passwords::AccessControlOptions;

        let access_control = SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
            AccessControlOptions::PRIVATE_KEY_USAGE.bits(),
        )
        .map_err(|err| anyhow!("Failed creating Secure Enclave access control: {err}"))?;

        let mut options = GenerateKeyOptions::default();
        options
            .set_key_type(KeyType::ec_sec_prime_random())
            .set_size_in_bits(256)
            .set_label(DPOP_SECURE_ENCLAVE_LABEL)
            .set_token(Token::SecureEnclave)
            .set_location(Location::DataProtectionKeychain)
            .set_access_control(access_control);

        let private_key =
            SecKey::new(&options).map_err(|err: CFError| classify_key_creation_error(err))?;

        Ok(Self { private_key })
    }

    pub(super) fn public_jwk(&self) -> Result<DpopPublicJwk> {
        let public_key = self
            .private_key
            .public_key()
            .context("Secure Enclave key has no public key")?;
        let bytes = public_key
            .external_representation()
            .context("Failed exporting Secure Enclave public key")?
            .to_vec();

        if bytes.len() != 65 || bytes[0] != 0x04 {
            bail!(
                "Unexpected Secure Enclave public key format (expected uncompressed P-256 point)"
            );
        }

        let x = &bytes[1..33];
        let y = &bytes[33..65];

        Ok(DpopPublicJwk {
            kty: "EC".to_string(),
            crv: "P-256".to_string(),
            x: URL_SAFE_NO_PAD.encode(x),
            y: URL_SAFE_NO_PAD.encode(y),
        })
    }

    pub(super) fn sign_dpop_input(&self, signing_input: &[u8]) -> Result<[u8; 64]> {
        use security_framework::key::Algorithm;

        let der_signature = self
            .private_key
            .create_signature(Algorithm::ECDSASignatureMessageX962SHA256, signing_input)
            .map_err(|err| anyhow!("Failed signing DPoP proof with Secure Enclave key: {err}"))?;

        der_signature_to_raw_ecdsa(&der_signature)
    }
}

#[cfg(target_os = "macos")]
fn classify_key_creation_error(err: core_foundation::error::CFError) -> anyhow::Error {
    match i32::try_from(err.code()).unwrap_or_default() {
        ERR_SEC_MISSING_ENTITLEMENT => anyhow!(
            "Failed creating Secure Enclave private key: missing required macOS entitlements for Data Protection Keychain access ({})",
            err.description()
        ),
        _ => anyhow!(
            "Failed creating Secure Enclave private key: {}",
            err.description()
        ),
    }
}
