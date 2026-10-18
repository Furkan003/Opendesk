//! Private connection invitations. Never publish an invitation: it contains the pairing key.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Invite {
    pub endpoint: String,
    pub relay: bool,
    pub secret: String,
}

fn validate(invite: &Invite) -> Result<(), String> {
    if invite.endpoint.is_empty()
        || invite.endpoint.len() > 255
        || invite
            .endpoint
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        || !invite.endpoint.contains(':')
        || invite.endpoint.contains(['/', '@'])
    {
        return Err("Invitation requires a host:port endpoint".into());
    }
    let code: String = invite.secret.chars().filter(|c| *c != '-').collect();
    let is_hex = code.len() == 64 && code.bytes().all(|b| b.is_ascii_hexdigit());
    let is_code = code.len() == 20
        && code
            .bytes()
            .all(|b| b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(&b.to_ascii_uppercase()));
    if !is_hex && !is_code {
        return Err("Invalid pairing code in invitation".into());
    }
    Ok(())
}

pub fn encode(endpoint: &str, relay: bool, secret: &str) -> Result<String, String> {
    let invite = Invite {
        endpoint: endpoint.to_owned(),
        relay,
        secret: secret.to_owned(),
    };
    validate(&invite)?;
    Ok(format!(
        "od2:{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&invite).map_err(|e| e.to_string())?)
    ))
}

pub fn decode(code: &str) -> Result<Invite, String> {
    if code.len() > 4096 {
        return Err("Invitation exceeds size limit".into());
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(
            code.trim()
                .strip_prefix("od2:")
                .ok_or("Not an Opendesk v2 invitation")?,
        )
        .map_err(|_| "Invalid invitation encoding")?;
    let invite: Invite = serde_json::from_slice(&bytes).map_err(|_| "Invalid invitation data")?;
    validate(&invite)?;
    Ok(invite)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invite_round_trip_and_input_validation() {
        let code = encode("192.168.1.2:7878", false, &"ab".repeat(32)).unwrap();
        assert_eq!(decode(&code).unwrap().endpoint, "192.168.1.2:7878");
        assert!(encode("bad/path:7878", true, &"ab".repeat(32)).is_err());
        assert!(encode("host:7878", true, "123456").is_err());
        assert!(decode("od2:invalid").is_err());
    }
}
