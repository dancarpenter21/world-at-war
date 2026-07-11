use std::{path::Path, sync::Arc};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const COOKIE_NAME: &str = "world_at_war_space_track";
const KEY_PATH: &str = "data/cache/space-track/credential-cookie.key";
const MAX_AGE_SECONDS: u64 = 60 * 60 * 24 * 30;

#[derive(Clone)]
pub struct CredentialCookie {
    cipher: Arc<ChaCha20Poly1305>,
    secure: bool,
}

#[derive(Serialize, Deserialize)]
pub struct RememberedCredentials {
    pub username: String,
    pub password: String,
}

impl CredentialCookie {
    pub async fn load() -> anyhow::Result<Self> {
        let key = match tokio::fs::read(KEY_PATH).await {
            Ok(bytes) if bytes.len() == 32 => bytes,
            _ => {
                let mut bytes = Vec::with_capacity(32);
                bytes.extend_from_slice(Uuid::new_v4().as_bytes());
                bytes.extend_from_slice(Uuid::new_v4().as_bytes());
                if let Some(parent) = Path::new(KEY_PATH).parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(KEY_PATH, &bytes).await?;
                bytes
            }
        };
        let secure = std::env::var("COOKIE_SECURE")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        Ok(Self {
            cipher: Arc::new(ChaCha20Poly1305::new_from_slice(&key)?),
            secure,
        })
    }

    pub fn seal(&self, credentials: &RememberedCredentials) -> anyhow::Result<String> {
        let nonce_id = Uuid::new_v4();
        let nonce = &nonce_id.as_bytes()[..12];
        let encrypted = self
            .cipher
            .encrypt(
                Nonce::from_slice(nonce),
                serde_json::to_vec(credentials)?.as_ref(),
            )
            .map_err(|_| anyhow::anyhow!("could not encrypt credential cookie"))?;
        let mut payload = nonce.to_vec();
        payload.extend_from_slice(&encrypted);
        Ok(URL_SAFE_NO_PAD.encode(payload))
    }

    pub fn open(&self, cookie_header: Option<&str>) -> Option<RememberedCredentials> {
        let encoded = cookie_header?
            .split(';')
            .filter_map(|item| item.trim().split_once('='))
            .find_map(|(name, value)| (name == COOKIE_NAME).then_some(value))?;
        let payload = URL_SAFE_NO_PAD.decode(encoded).ok()?;
        if payload.len() <= 12 {
            return None;
        }
        let decrypted = self
            .cipher
            .decrypt(Nonce::from_slice(&payload[..12]), &payload[12..])
            .ok()?;
        serde_json::from_slice(&decrypted).ok()
    }

    pub fn set_header(&self, value: &str) -> String {
        format!(
            "{COOKIE_NAME}={value}; Path=/v1; Max-Age={MAX_AGE_SECONDS}; HttpOnly; SameSite=Strict{}",
            if self.secure { "; Secure" } else { "" }
        )
    }

    pub fn clear_header(&self) -> String {
        format!(
            "{COOKIE_NAME}=; Path=/v1; Max-Age=0; HttpOnly; SameSite=Strict{}",
            if self.secure { "; Secure" } else { "" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cookie() -> CredentialCookie {
        CredentialCookie {
            cipher: Arc::new(ChaCha20Poly1305::new_from_slice(&[7_u8; 32]).unwrap()),
            secure: false,
        }
    }

    #[test]
    fn encrypts_and_recovers_credentials_without_plaintext_cookie_data() {
        let cookie = test_cookie();
        let credentials = RememberedCredentials {
            username: "operator@example.test".into(),
            password: "not-a-real-password".into(),
        };
        let sealed = cookie.seal(&credentials).unwrap();
        assert!(!sealed.contains(&credentials.username));
        assert!(!sealed.contains(&credentials.password));

        let header = format!("another=value; {COOKIE_NAME}={sealed}");
        let restored = cookie.open(Some(&header)).unwrap();
        assert_eq!(restored.username, credentials.username);
        assert_eq!(restored.password, credentials.password);
    }

    #[test]
    fn rejects_modified_cookie_ciphertext() {
        let cookie = test_cookie();
        let credentials = RememberedCredentials {
            username: "operator@example.test".into(),
            password: "not-a-real-password".into(),
        };
        let mut sealed = cookie.seal(&credentials).unwrap();
        sealed.push('A');
        let header = format!("{COOKIE_NAME}={sealed}");
        assert!(cookie.open(Some(&header)).is_none());
    }
}
