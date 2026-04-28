use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct DecodedToken {
    pub person_id: String,
    pub expires_at: Option<i64>,
    pub claims: Value,
}

#[derive(Debug, Deserialize)]
struct ClaimsPayload {
    #[serde(default)]
    exp: Option<i64>,
    #[serde(default)]
    person_id: Option<String>,
    #[serde(rename = "https://de.scalable.capital/person_id")]
    #[serde(default)]
    ns_person_id: Option<String>,
    #[serde(rename = "https://de.scalable.capital/personId")]
    #[serde(default)]
    ns_person_id_camel: Option<String>,
}

pub fn decode_unverified(token: &str) -> Result<DecodedToken> {
    decode_untrusted_for_diagnostics(token)
}

pub fn decode_untrusted_for_diagnostics(token: &str) -> Result<DecodedToken> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        bail!("Token is not a valid JWT");
    }

    let payload = URL_SAFE_NO_PAD
        .decode(parts[1])
        .context("Failed to decode JWT payload")?;

    let decoded_claims: Value =
        serde_json::from_slice(&payload).context("JWT payload is not valid JSON")?;
    let claims_object = decoded_claims
        .as_object()
        .cloned()
        .context("JWT payload is not a JSON object")?;
    let claims: ClaimsPayload =
        serde_json::from_value(decoded_claims).context("JWT payload has unexpected claim types")?;

    let person_id = claims
        .person_id
        .or(claims.ns_person_id)
        .or(claims.ns_person_id_camel)
        .context("Token does not contain person_id claim")?;

    Ok(DecodedToken {
        person_id,
        expires_at: claims.exp,
        claims: Value::Object(claims_object),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn make_token(payload: &str) -> String {
        let header = r#"{"alg":"none","typ":"JWT"}"#;
        let h = URL_SAFE_NO_PAD.encode(header);
        let p = URL_SAFE_NO_PAD.encode(payload);
        format!("{h}.{p}.sig")
    }

    #[test]
    fn extracts_plain_person_id() {
        let token = make_token(r#"{"person_id":"abc","exp":123}"#);
        let decoded = decode_untrusted_for_diagnostics(&token).unwrap();
        assert_eq!(decoded.person_id, "abc");
        assert_eq!(decoded.expires_at, Some(123));
        assert_eq!(decoded.claims["person_id"], "abc");
        assert_eq!(decoded.claims["exp"], 123);
    }

    #[test]
    fn claims_preserve_typed_and_unknown_fields() {
        let token = make_token(
            r#"{"person_id":"abc","exp":123,"sub":"user-1","https://de.scalable.capital/personId":"camel"}"#,
        );
        let decoded = decode_untrusted_for_diagnostics(&token).unwrap();

        assert_eq!(decoded.claims["person_id"], "abc");
        assert_eq!(decoded.claims["exp"], 123);
        assert_eq!(decoded.claims["sub"], "user-1");
        assert_eq!(
            decoded.claims["https://de.scalable.capital/personId"],
            "camel"
        );
    }

    #[test]
    fn extracts_namespaced_person_id() {
        let token = make_token(r#"{"https://de.scalable.capital/person_id":"xyz"}"#);
        let decoded = decode_untrusted_for_diagnostics(&token).unwrap();
        assert_eq!(decoded.person_id, "xyz");
    }

    #[test]
    fn extracts_namespaced_person_id_camel_case() {
        let token = make_token(r#"{"https://de.scalable.capital/personId":"uvw"}"#);
        let decoded = decode_untrusted_for_diagnostics(&token).unwrap();
        assert_eq!(decoded.person_id, "uvw");
    }

    #[test]
    fn fails_without_person_id() {
        let token = make_token(r#"{"sub":"foo"}"#);
        assert!(decode_untrusted_for_diagnostics(&token).is_err());
    }

    #[test]
    fn invalid_json_reports_json_context() {
        let token = make_token(r#"{"person_id":"abc""#);
        let err = decode_untrusted_for_diagnostics(&token).unwrap_err();

        assert!(
            err.to_string().contains("JWT payload is not valid JSON"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn invalid_claim_type_reports_schema_context() {
        let token = make_token(r#"{"person_id":"abc","exp":"soon"}"#);
        let err = decode_untrusted_for_diagnostics(&token).unwrap_err();

        assert!(
            err.to_string()
                .contains("JWT payload has unexpected claim types"),
            "unexpected error: {err:#}"
        );
    }
}
