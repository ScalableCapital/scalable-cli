use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
#[cfg(target_os = "linux")]
use sha2::{Digest, Sha256};

use super::{DpopPublicJwk, der_signature_to_raw_ecdsa};
#[cfg(target_os = "linux")]
use crate::config::Pkcs11Config;

#[cfg(target_os = "linux")]
use cryptoki::context::{CInitializeArgs, CInitializeFlags, Pkcs11};
#[cfg(target_os = "linux")]
use cryptoki::error::{Error as CryptokiError, RvError};
#[cfg(target_os = "linux")]
use cryptoki::mechanism::Mechanism;
#[cfg(target_os = "linux")]
use cryptoki::object::{Attribute, AttributeType, KeyType, ObjectClass, ObjectHandle};
#[cfg(target_os = "linux")]
use cryptoki::session::{Session, UserType};
#[cfg(target_os = "linux")]
use cryptoki::slot::{Slot, TokenInfo};
#[cfg(target_os = "linux")]
use cryptoki::types::AuthPin;

#[cfg(target_os = "linux")]
const PKCS11_PIN_ENV_VAR: &str = "SC_PKCS11_PIN";
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
const P256_EC_PARAMS_DER: &[u8] = &[0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub(super) struct Pkcs11Key {
    pkcs11: Pkcs11,
    slot: Slot,
    key_uri: Pkcs11KeyUri,
    public_jwk: DpopPublicJwk,
}

#[cfg(target_os = "linux")]
impl Pkcs11Key {
    pub(super) fn load(config: &Pkcs11Config) -> Result<Self> {
        let module_path = config.module_path.trim();
        if module_path.is_empty() {
            bail!("PKCS#11 module_path must be non-empty");
        }

        let key_uri = Pkcs11KeyUri::parse(&config.key_uri)?;
        let pkcs11 = Pkcs11::new(module_path)
            .with_context(|| format!("Failed to load PKCS#11 module at {module_path}"))?;
        match pkcs11.initialize(CInitializeArgs::new(CInitializeFlags::OS_LOCKING_OK)) {
            Ok(()) => {}
            Err(CryptokiError::Pkcs11(RvError::CryptokiAlreadyInitialized, _)) => {}
            Err(err) => return Err(err).context("Failed to initialize PKCS#11 module"),
        }

        let slot = select_pkcs11_slot(&pkcs11, &key_uri)?;
        let public_jwk = with_pkcs11_session(&pkcs11, slot, |session| {
            login_pkcs11_session_if_needed(session, &pkcs11.get_token_info(slot)?)?;
            load_pkcs11_public_jwk(session, &key_uri)
        })?;

        Ok(Self {
            pkcs11,
            slot,
            key_uri,
            public_jwk,
        })
    }

    pub(super) fn public_jwk(&self) -> Result<DpopPublicJwk> {
        Ok(self.public_jwk.clone())
    }

    pub(super) fn sign_dpop_input(&self, signing_input: &[u8]) -> Result<[u8; 64]> {
        let digest = Sha256::digest(signing_input);
        with_pkcs11_session(&self.pkcs11, self.slot, |session| {
            let token_info = self.pkcs11.get_token_info(self.slot)?;
            login_pkcs11_session_if_needed(session, &token_info)?;
            let private_key = find_pkcs11_private_key(session, &self.key_uri)?;
            let signature = session
                .sign(&Mechanism::Ecdsa, private_key, &digest)
                .context("PKCS#11 signing operation failed")?;
            normalize_pkcs11_ecdsa_signature(&signature)
        })
    }
}

#[cfg(target_os = "linux")]
fn with_pkcs11_session<T, F>(pkcs11: &Pkcs11, slot: Slot, f: F) -> Result<T>
where
    F: FnOnce(&Session) -> Result<T>,
{
    let session = pkcs11
        .open_ro_session(slot)
        .context("Failed to open PKCS#11 session")?;
    f(&session)
}

#[cfg(target_os = "linux")]
fn select_pkcs11_slot(pkcs11: &Pkcs11, key_uri: &Pkcs11KeyUri) -> Result<Slot> {
    let slots = pkcs11
        .get_slots_with_token()
        .context("Failed to enumerate PKCS#11 slots with tokens")?;

    let mut matches = Vec::new();
    for slot in slots {
        let token_info = pkcs11.get_token_info(slot).with_context(|| {
            format!("Failed to read PKCS#11 token metadata for slot {:?}", slot)
        })?;
        if key_uri.matches_token(&token_info)? {
            matches.push(slot);
        }
    }

    match matches.len() {
        1 => Ok(matches[0]),
        0 => bail!("PKCS#11 key_uri did not match any available token"),
        _ => bail!(
            "PKCS#11 key_uri matched multiple tokens; add token= or serial= to make it unambiguous"
        ),
    }
}

#[cfg(target_os = "linux")]
fn login_pkcs11_session_if_needed(session: &Session, token_info: &TokenInfo) -> Result<()> {
    if !token_info.login_required() {
        return Ok(());
    }

    if token_info.protected_authentication_path() {
        handle_pkcs11_login_result(
            session.login(UserType::User, None),
            "PKCS#11 login via protected authentication path failed",
        )?;
        return Ok(());
    }

    let mut pin = std::env::var(PKCS11_PIN_ENV_VAR).unwrap_or_default();
    trim_string_in_place(&mut pin);
    if pin.is_empty() {
        bail!(
            "PKCS#11 token login requires a PIN; set {PKCS11_PIN_ENV_VAR} or use a token with protected authentication path"
        );
    }

    let pin = AuthPin::new(pin.into_boxed_str());
    handle_pkcs11_login_result(
        session.login(UserType::User, Some(&pin)),
        &format!("PKCS#11 login failed using {PKCS11_PIN_ENV_VAR}"),
    )?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn handle_pkcs11_login_result(
    login_result: std::result::Result<(), CryptokiError>,
    context_message: &str,
) -> Result<()> {
    match login_result {
        Ok(()) | Err(CryptokiError::Pkcs11(RvError::UserAlreadyLoggedIn, _)) => Ok(()),
        Err(error) => Err(error).with_context(|| context_message.to_string()),
    }
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn trim_string_in_place(value: &mut String) {
    let original_len = value.len();
    let start = value
        .find(|c: char| !c.is_whitespace())
        .unwrap_or(original_len);
    let end = value
        .rfind(|c: char| !c.is_whitespace())
        .map(|index| index + 1)
        .unwrap_or(start);

    if start == end {
        value.clear();
        return;
    }

    if start > 0 {
        value.drain(..start);
    }

    if end < original_len {
        value.truncate(end - start);
    }
}

#[cfg(target_os = "linux")]
fn load_pkcs11_public_jwk(session: &Session, key_uri: &Pkcs11KeyUri) -> Result<DpopPublicJwk> {
    let public_key = find_pkcs11_public_key(session, key_uri)?;
    let attributes = session
        .get_attributes(
            public_key,
            &[AttributeType::EcParams, AttributeType::EcPoint],
        )
        .context("Failed reading PKCS#11 public key attributes")?;

    let mut ec_params = None;
    let mut ec_point = None;

    for attribute in attributes {
        match attribute {
            Attribute::EcParams(params) => ec_params = Some(params),
            Attribute::EcPoint(point) => ec_point = Some(point),
            _ => {}
        }
    }

    let ec_params = ec_params.context("PKCS#11 public key is missing EC parameters")?;
    let ec_point = ec_point.context("PKCS#11 public key is missing EC point")?;

    parse_pkcs11_public_jwk(&ec_params, &ec_point)
}

#[cfg(target_os = "linux")]
fn find_pkcs11_private_key(session: &Session, key_uri: &Pkcs11KeyUri) -> Result<ObjectHandle> {
    find_pkcs11_key_object(session, key_uri, ObjectClass::PRIVATE_KEY)
}

#[cfg(target_os = "linux")]
fn find_pkcs11_public_key(session: &Session, key_uri: &Pkcs11KeyUri) -> Result<ObjectHandle> {
    find_pkcs11_key_object(session, key_uri, ObjectClass::PUBLIC_KEY)
}

#[cfg(target_os = "linux")]
fn find_pkcs11_key_object(
    session: &Session,
    key_uri: &Pkcs11KeyUri,
    class: ObjectClass,
) -> Result<ObjectHandle> {
    let template = key_uri.object_search_template(class);
    let objects = session
        .find_objects(&template)
        .context("Failed searching PKCS#11 objects")?;

    match objects.len() {
        1 => Ok(objects[0]),
        0 if class == ObjectClass::PRIVATE_KEY => {
            bail!("PKCS#11 key_uri did not resolve to a matching EC private key object")
        }
        0 if class == ObjectClass::PUBLIC_KEY => bail!(
            "PKCS#11 v1 requires a matching EC public key object; certificate fallback is not supported yet"
        ),
        0 => bail!("PKCS#11 object search returned no match"),
        _ => bail!("PKCS#11 key_uri matched multiple objects; use a more specific id= or object="),
    }
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_pkcs11_public_jwk(ec_params: &[u8], ec_point: &[u8]) -> Result<DpopPublicJwk> {
    if ec_params != P256_EC_PARAMS_DER {
        bail!("Unsupported PKCS#11 EC curve; expected P-256");
    }

    let encoded_point = parse_pkcs11_ec_point(ec_point)?;
    if encoded_point.len() != 65 || encoded_point[0] != 0x04 {
        bail!("Unsupported PKCS#11 EC point format; expected uncompressed P-256 point");
    }

    Ok(DpopPublicJwk {
        kty: "EC".to_string(),
        crv: "P-256".to_string(),
        x: URL_SAFE_NO_PAD.encode(&encoded_point[1..33]),
        y: URL_SAFE_NO_PAD.encode(&encoded_point[33..65]),
    })
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_pkcs11_ec_point(value: &[u8]) -> Result<Vec<u8>> {
    if value.len() == 65 && value[0] == 0x04 {
        return Ok(value.to_vec());
    }

    let Some((&tag, rest)) = value.split_first() else {
        bail!("PKCS#11 EC point is empty");
    };
    if tag != 0x04 {
        bail!("Unsupported PKCS#11 EC point encoding");
    }

    let (content_len, content_offset) = parse_der_length(rest)?;
    let content_end = content_offset
        .checked_add(content_len)
        .context("Invalid DER-wrapped PKCS#11 EC point")?;
    let encoded_point = rest
        .get(content_offset..content_end)
        .context("Invalid DER-wrapped PKCS#11 EC point")?;
    Ok(encoded_point.to_vec())
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn parse_der_length(bytes: &[u8]) -> Result<(usize, usize)> {
    let Some((&first, rest)) = bytes.split_first() else {
        bail!("Missing DER length");
    };

    if first & 0x80 == 0 {
        return Ok((usize::from(first), 1));
    }

    let len_len = usize::from(first & 0x7f);
    if len_len == 0 || len_len > std::mem::size_of::<usize>() {
        bail!("Unsupported DER length encoding");
    }

    let len_bytes = rest
        .get(..len_len)
        .context("Truncated DER length encoding")?;
    let mut length = 0usize;
    for byte in len_bytes {
        length = (length << 8) | usize::from(*byte);
    }

    Ok((length, 1 + len_len))
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn normalize_pkcs11_ecdsa_signature(signature: &[u8]) -> Result<[u8; 64]> {
    if signature.len() == 64 {
        let mut raw = [0_u8; 64];
        raw.copy_from_slice(signature);
        return Ok(raw);
    }

    der_signature_to_raw_ecdsa(signature)
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
#[derive(Debug, Clone)]
struct Pkcs11KeyUri {
    token: Option<String>,
    serial: Option<String>,
    manufacturer: Option<String>,
    model: Option<String>,
    object_label: Option<String>,
    id: Option<Vec<u8>>,
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
impl Pkcs11KeyUri {
    fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        let attrs = raw
            .strip_prefix("pkcs11:")
            .context("PKCS#11 key_uri must start with pkcs11:")?;

        if attrs.is_empty() {
            bail!("PKCS#11 key_uri must include object selectors");
        }
        if attrs.contains('?') {
            bail!("PKCS#11 key_uri query parameters are not supported in v1");
        }

        let mut key_uri = Self {
            token: None,
            serial: None,
            manufacturer: None,
            model: None,
            object_label: None,
            id: None,
        };

        for part in attrs.split(';') {
            if part.is_empty() {
                continue;
            }

            let (name, value) = part
                .split_once('=')
                .with_context(|| format!("Invalid PKCS#11 key_uri attribute '{part}'"))?;
            let value_bytes = percent_decode_pkcs11_bytes(value)?;

            match name {
                "token" => set_pkcs11_text_attr(&mut key_uri.token, name, value_bytes)?,
                "serial" => set_pkcs11_text_attr(&mut key_uri.serial, name, value_bytes)?,
                "manufacturer" => {
                    set_pkcs11_text_attr(&mut key_uri.manufacturer, name, value_bytes)?
                }
                "model" => set_pkcs11_text_attr(&mut key_uri.model, name, value_bytes)?,
                "object" => set_pkcs11_text_attr(&mut key_uri.object_label, name, value_bytes)?,
                "id" => set_pkcs11_binary_attr(&mut key_uri.id, name, value_bytes)?,
                "type" => {
                    let value = String::from_utf8(value_bytes)
                        .context("PKCS#11 type attribute must be valid UTF-8")?;
                    if value != "private" {
                        bail!("PKCS#11 key_uri type must be 'private' or omitted");
                    }
                }
                unsupported => {
                    bail!("Unsupported PKCS#11 key_uri attribute '{unsupported}' in v1")
                }
            }
        }

        if key_uri.object_label.is_none() && key_uri.id.is_none() {
            bail!("PKCS#11 key_uri must include id= or object= to identify the key");
        }

        Ok(key_uri)
    }

    #[cfg(target_os = "linux")]
    fn matches_token(&self, token_info: &TokenInfo) -> Result<bool> {
        Ok(
            matches_pkcs11_token_attr(self.token.as_deref(), token_info.label())
                && matches_pkcs11_token_attr(self.serial.as_deref(), token_info.serial_number())
                && matches_pkcs11_token_attr(
                    self.manufacturer.as_deref(),
                    token_info.manufacturer_id(),
                )
                && matches_pkcs11_token_attr(self.model.as_deref(), token_info.model()),
        )
    }

    #[cfg(target_os = "linux")]
    fn object_search_template(&self, class: ObjectClass) -> Vec<Attribute> {
        let mut template = vec![Attribute::Class(class), Attribute::KeyType(KeyType::EC)];

        if let Some(id) = &self.id {
            template.push(Attribute::Id(id.clone()));
        }
        if let Some(label) = &self.object_label {
            template.push(Attribute::Label(label.as_bytes().to_vec()));
        }

        template
    }
}

#[cfg(target_os = "linux")]
fn matches_pkcs11_token_attr(expected: Option<&str>, actual: &str) -> bool {
    expected.is_none_or(|expected| normalize_pkcs11_token_text(actual) == expected)
}

#[cfg(target_os = "linux")]
fn normalize_pkcs11_token_text(value: &str) -> &str {
    value.trim_end()
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn set_pkcs11_text_attr(slot: &mut Option<String>, name: &str, value_bytes: Vec<u8>) -> Result<()> {
    if slot.is_some() {
        bail!("Duplicate PKCS#11 key_uri attribute '{name}'");
    }

    let value =
        String::from_utf8(value_bytes).with_context(|| format!("PKCS#11 {name} must be UTF-8"))?;
    if value.is_empty() {
        bail!("PKCS#11 key_uri attribute '{name}' must be non-empty");
    }
    *slot = Some(value);
    Ok(())
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn set_pkcs11_binary_attr(
    slot: &mut Option<Vec<u8>>,
    name: &str,
    value_bytes: Vec<u8>,
) -> Result<()> {
    if slot.is_some() {
        bail!("Duplicate PKCS#11 key_uri attribute '{name}'");
    }
    if value_bytes.is_empty() {
        bail!("PKCS#11 key_uri attribute '{name}' must be non-empty");
    }
    *slot = Some(value_bytes);
    Ok(())
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn percent_decode_pkcs11_bytes(value: &str) -> Result<Vec<u8>> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut chars = value.as_bytes().iter().copied();

    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let high = chars
                .next()
                .context("Truncated percent-encoding in PKCS#11 key_uri")?;
            let low = chars
                .next()
                .context("Truncated percent-encoding in PKCS#11 key_uri")?;
            let high = decode_hex_nibble(high)?;
            let low = decode_hex_nibble(low)?;
            decoded.push((high << 4) | low);
        } else {
            decoded.push(byte);
        }
    }

    Ok(decoded)
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn decode_hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => bail!("Invalid percent-encoding in PKCS#11 key_uri"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn handle_pkcs11_login_result_accepts_already_logged_in() {
        let result = handle_pkcs11_login_result(
            Err(CryptokiError::Pkcs11(
                RvError::UserAlreadyLoggedIn,
                cryptoki::context::Function::Login,
            )),
            "login failed",
        );

        assert!(result.is_ok());
    }

    #[test]
    fn pkcs11_signature_normalization_accepts_raw_ecdsa_output() {
        let mut expected = [0_u8; 64];
        for (index, byte) in expected.iter_mut().enumerate() {
            *byte = index as u8;
        }

        let normalized =
            normalize_pkcs11_ecdsa_signature(&expected).expect("normalize raw signature");
        assert_eq!(normalized, expected);
    }

    #[test]
    fn trim_string_in_place_removes_surrounding_whitespace_without_copying_shape() {
        let mut value = " \t 1234 \n".to_string();
        trim_string_in_place(&mut value);
        assert_eq!(value, "1234");

        let mut whitespace_only = " \n\t ".to_string();
        trim_string_in_place(&mut whitespace_only);
        assert!(whitespace_only.is_empty());
    }

    #[test]
    fn pkcs11_key_uri_parses_object_and_binary_id() {
        let parsed = Pkcs11KeyUri::parse("pkcs11:token=YubiKey%20PIV;object=Auth%20Key;id=%01%02")
            .expect("parse key uri");

        assert_eq!(parsed.token.as_deref(), Some("YubiKey PIV"));
        assert_eq!(parsed.object_label.as_deref(), Some("Auth Key"));
        assert_eq!(parsed.id, Some(vec![0x01, 0x02]));
    }

    #[test]
    fn pkcs11_key_uri_rejects_query_parameters() {
        let err =
            Pkcs11KeyUri::parse("pkcs11:token=YubiKey%20PIV;id=%01?pin-value=1234").unwrap_err();

        assert!(
            err.to_string()
                .contains("query parameters are not supported")
        );
    }

    #[test]
    fn pkcs11_public_jwk_parses_der_wrapped_ec_point() {
        let jwk = parse_pkcs11_public_jwk(
            P256_EC_PARAMS_DER,
            &[
                0x04, 0x41, 0x04, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
                0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
                0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
                0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35,
                0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40,
            ],
        )
        .expect("parse jwk");

        assert_eq!(jwk.kty, "EC");
        assert_eq!(jwk.crv, "P-256");
        assert_eq!(
            jwk.x,
            URL_SAFE_NO_PAD.encode([
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
                0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
                0x1d, 0x1e, 0x1f, 0x20,
            ])
        );
        assert_eq!(
            jwk.y,
            URL_SAFE_NO_PAD.encode([
                0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e,
                0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c,
                0x3d, 0x3e, 0x3f, 0x40,
            ])
        );
    }
}
