use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;
use uuid::Uuid;

const SESSION_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;
const MIN_ITERATIONS: u32 = 10_000;
const MAX_ITERATIONS: u32 = 2_000_000;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct AuthSettings {
    username: String,
    iterations: u32,
    salt: Vec<u8>,
    digest: Vec<u8>,
    session_secret: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum AuthConfigError {
    #[error("authentication requires OP_ANI_AUTH_USER and OP_ANI_AUTH_PASSWORD_HASH")]
    Missing,
    #[error("authentication username is invalid")]
    InvalidUsername,
    #[error("authentication password hash is invalid")]
    InvalidPasswordHash,
}

impl AuthSettings {
    pub fn new(
        username: impl Into<String>,
        password_hash: impl Into<String>,
        secret: Option<String>,
    ) -> Result<Self, AuthConfigError> {
        let username = username.into().trim().to_string();
        let password_hash = password_hash.into().trim().to_string();
        if username.is_empty() || username.contains('|') || username.chars().count() > 100 {
            return Err(AuthConfigError::InvalidUsername);
        }

        let mut parts = password_hash.split('$');
        if parts.next() != Some("pbkdf2_sha256") {
            return Err(AuthConfigError::InvalidPasswordHash);
        }
        let iterations = parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| (MIN_ITERATIONS..=MAX_ITERATIONS).contains(value))
            .ok_or(AuthConfigError::InvalidPasswordHash)?;
        let salt = parts
            .next()
            .and_then(decode_base64url)
            .ok_or(AuthConfigError::InvalidPasswordHash)?;
        let digest = parts
            .next()
            .and_then(decode_base64url)
            .filter(|value| value.len() == 32)
            .ok_or(AuthConfigError::InvalidPasswordHash)?;
        if parts.next().is_some() {
            return Err(AuthConfigError::InvalidPasswordHash);
        }

        let session_secret = secret
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| password_hash.clone())
            .into_bytes();

        Ok(Self {
            username,
            iterations,
            salt,
            digest,
            session_secret,
        })
    }

    pub fn from_env() -> Result<Self, AuthConfigError> {
        let username = env::var("OP_ANI_AUTH_USER").unwrap_or_default();
        let password_hash = env::var("OP_ANI_AUTH_PASSWORD_HASH").unwrap_or_default();
        if username.trim().is_empty() || password_hash.trim().is_empty() {
            return Err(AuthConfigError::Missing);
        }
        let secret = env::var("OP_ANI_AUTH_SECRET").ok();
        Self::new(username, password_hash, secret)
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn verify_password(&self, password: &str) -> bool {
        let mut actual = [0_u8; 32];
        pbkdf2_hmac::<Sha256>(
            password.as_bytes(),
            &self.salt,
            self.iterations,
            &mut actual,
        );
        actual.ct_eq(self.digest.as_slice()).into()
    }

    pub fn issue_session(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        self.issue_session_at(now)
    }

    pub fn session_username(&self, token: &str) -> Option<&str> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        self.session_username_at(token, now)
    }

    pub fn session_username_at(&self, token: &str, now: u64) -> Option<&str> {
        let (payload, signature) = token.rsplit_once('.')?;
        let signature = decode_base64url(signature)?;
        let mut fields = payload.splitn(3, '|');
        let username = fields.next()?;
        let expires_at = fields.next()?.parse::<u64>().ok()?;
        fields.next()?;
        if expires_at <= now || username != self.username {
            return None;
        }
        let mut mac = HmacSha256::new_from_slice(&self.session_secret).ok()?;
        mac.update(payload.as_bytes());
        mac.verify_slice(&signature).ok()?;
        Some(&self.username)
    }

    fn issue_session_at(&self, now: u64) -> String {
        let expires_at = now.saturating_add(SESSION_TTL_SECONDS);
        let nonce = Uuid::new_v4().simple().to_string();
        let payload = format!("{}|{expires_at}|{nonce}", self.username);
        let mut mac = HmacSha256::new_from_slice(&self.session_secret)
            .expect("HMAC accepts keys of any size");
        mac.update(payload.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{payload}.{signature}")
    }
}

fn decode_base64url(value: &str) -> Option<Vec<u8>> {
    let unpadded = value.trim_end_matches('=');
    URL_SAFE_NO_PAD.decode(unpadded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_falls_back_as_session_secret() {
        let hash = "pbkdf2_sha256$10000$dGVzdA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let auth = AuthSettings::new("user", hash, None).unwrap();
        assert_eq!(auth.session_secret, hash.as_bytes());
    }

    #[test]
    fn malformed_or_out_of_range_password_hash_is_rejected_at_startup() {
        assert!(AuthSettings::new("user", "plain-text", None).is_err());
        assert!(
            AuthSettings::new(
                "user",
                "pbkdf2_sha256$9999$dGVzdA$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                None,
            )
            .is_err()
        );
    }
}
