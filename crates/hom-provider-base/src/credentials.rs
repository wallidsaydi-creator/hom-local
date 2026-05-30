use std::process::Command;

use crate::ProviderError;

pub fn resolve_api_key(
    provider_id: &str,
    inline_api_key: Option<String>,
    credential_ref: Option<String>,
    env_var: &str,
) -> Result<String, ProviderError> {
    if let Some(key) = inline_api_key.filter(|key| !key.trim().is_empty()) {
        return Ok(key);
    }
    if let Some(reference) = credential_ref.filter(|reference| !reference.trim().is_empty()) {
        return read_keychain_secret(&reference).map_err(|error| {
            ProviderError::new(
                "credential_ref_unavailable",
                format!("{provider_id} credential reference is unavailable: {error}"),
                false,
            )
        });
    }
    std::env::var(env_var).map_err(|_| ProviderError::missing_key(provider_id))
}

pub fn resolve_oauth_access_token(
    provider_id: &str,
    oauth_ref: Option<String>,
) -> Result<String, ProviderError> {
    let reference = oauth_ref
        .filter(|reference| !reference.trim().is_empty())
        .ok_or_else(|| ProviderError::missing_key(provider_id))?;
    let payload = read_keychain_secret(&reference).map_err(|error| {
        ProviderError::new(
            "oauth_ref_unavailable",
            format!("{provider_id} OAuth reference is unavailable: {error}"),
            false,
        )
    })?;
    let value = parse_oauth_payload(&payload).ok_or_else(|| {
        ProviderError::new(
            "oauth_payload_invalid",
            format!("{provider_id} OAuth reference did not contain a valid token payload"),
            false,
        )
    })?;
    value
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| {
            ProviderError::new(
                "oauth_access_token_missing",
                format!("{provider_id} OAuth token payload did not include an access token"),
                false,
            )
        })
}

pub fn read_keychain_secret(reference: &str) -> Result<String, String> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", reference, "-w"])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("keychain_item_not_found".to_string());
    }
    let secret = String::from_utf8(output.stdout)
        .map_err(|_| "keychain_secret_not_utf8".to_string())?
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if secret.trim().is_empty() {
        return Err("keychain_secret_empty".to_string());
    }
    Ok(secret)
}

fn parse_oauth_payload(payload: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .or_else(|| decode_hex_json_payload(payload))
}

fn decode_hex_json_payload(payload: &str) -> Option<serde_json::Value> {
    let trimmed = payload.trim();
    if trimmed.len() < 2 || trimmed.len() % 2 != 0 {
        return None;
    }
    if !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = (0..trimmed.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&trimmed[index..index + 2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    let decoded = String::from_utf8(bytes).ok()?;
    let decoded = decoded.trim();
    if !decoded.starts_with('{') {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::parse_oauth_payload;

    #[test]
    fn oauth_payload_accepts_plain_json() {
        let value = parse_oauth_payload(r#"{"access_token":"plain-token"}"#).unwrap();
        assert_eq!(value["access_token"], "plain-token");
    }

    #[test]
    fn oauth_payload_accepts_hex_encoded_json_from_security_cli() {
        let json = r#"{"access_token":"hex-token"}"#;
        let hex = json
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let value = parse_oauth_payload(&hex).unwrap();
        assert_eq!(value["access_token"], "hex-token");
    }
}
