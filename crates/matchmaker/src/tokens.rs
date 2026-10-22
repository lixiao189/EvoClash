//! Credentials: netcode connect tokens and stateless reconnect keys.

use std::net::SocketAddr;

use base64::Engine;
use evoclash_game::net::{CONNECTION_TIMEOUT_SECS, PROTOCOL_ID, TOKEN_EXPIRE_SECS};
use hmac::{Hmac, Mac};
use lightyear::netcode::{ConnectToken, USER_DATA_BYTES};
use sha2::Sha256;

/// Signed token that lets exactly `client_id` into the match at `server`.
/// The player's display name rides along in the encrypted user data.
pub fn connect_token(
    key: &[u8; 32],
    server: SocketAddr,
    client_id: u64,
    name: &str,
) -> Result<String, String> {
    let mut user_data = [0u8; USER_DATA_BYTES];
    let name = truncate_utf8(name, 48);
    user_data[..name.len()].copy_from_slice(name.as_bytes());
    let token = ConnectToken::build(server, PROTOCOL_ID, client_id, *key)
        .expire_seconds(TOKEN_EXPIRE_SECS)
        .timeout_seconds(CONNECTION_TIMEOUT_SECS)
        .user_data(user_data)
        .generate()
        .map_err(|e| format!("token: {e:?}"))?
        .try_into_bytes()
        .map_err(|e| format!("token bytes: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(token))
}

fn truncate_utf8(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim()
}

/// HMAC over (match, client): any matchmaker replica holding the shared key can verify it,
/// so reconnects survive a matchmaker restart without shared session storage.
pub fn reconnect_key(secret: &[u8; 32], match_id: &str, client_id: u64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("any key length works");
    mac.update(b"evoclash-reconnect\0");
    mac.update(match_id.as_bytes());
    mac.update(&client_id.to_le_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn verify_reconnect_key(secret: &[u8; 32], match_id: &str, client_id: u64, key: &str) -> bool {
    let expected = reconnect_key(secret, match_id, client_id);
    // Constant-time comparison.
    expected.len() == key.len()
        && expected
            .bytes()
            .zip(key.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_keys_are_bound_to_match_and_client() {
        let secret = [7u8; 32];
        let key = reconnect_key(&secret, "m1", 42);
        assert!(verify_reconnect_key(&secret, "m1", 42, &key));
        assert!(!verify_reconnect_key(&secret, "m1", 43, &key));
        assert!(!verify_reconnect_key(&secret, "m2", 42, &key));
        assert!(!verify_reconnect_key(&[8u8; 32], "m1", 42, &key));
    }

    #[test]
    fn names_are_truncated_on_char_boundaries() {
        assert_eq!(truncate_utf8("幸存者幸存者", 7), "幸存");
    }

    #[test]
    fn tokens_round_trip() {
        let token =
            connect_token(&[1u8; 32], "127.0.0.1:7000".parse().unwrap(), 5, "Alice").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(token)
            .unwrap();
        assert!(ConnectToken::try_from_bytes(&bytes).is_ok());
    }
}
