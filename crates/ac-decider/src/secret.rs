//! Secrets at rest (decider API keys, webhook secrets): AES-256-GCM under the server master key.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

const PREFIX: &str = "v1:";

#[derive(Clone)]
pub struct SecretBox {
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBox(..)")
    }
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("OS randomness");
    b
}

impl SecretBox {
    /// `master_key` is 32 bytes, base64 encoded (`CREWCONVO_MASTER_KEY`).
    pub fn from_base64(master_key: &str) -> Result<Self, String> {
        let raw = B64.decode(master_key.trim()).map_err(|e| format!("master key is not base64: {e}"))?;
        if raw.len() != 32 {
            return Err(format!("master key must be 32 bytes, got {}", raw.len()));
        }
        Ok(SecretBox { cipher: Aes256Gcm::new_from_slice(&raw).map_err(|e| e.to_string())? })
    }

    /// A random key (tests / first run without configuration).
    pub fn ephemeral() -> Self {
        SecretBox { cipher: Aes256Gcm::new_from_slice(&random_bytes::<32>()).expect("key") }
    }

    pub fn encrypt(&self, plaintext: &str) -> String {
        let nonce = random_bytes::<12>();
        let ct = self.cipher.encrypt(&nonce.into(), plaintext.as_bytes()).expect("aes-gcm encrypt");
        let mut out = nonce.to_vec();
        out.extend(ct);
        format!("{PREFIX}{}", B64.encode(out))
    }

    pub fn decrypt(&self, sealed: &str) -> Result<String, String> {
        let raw = B64.decode(sealed.strip_prefix(PREFIX).ok_or("unknown secret format")?).map_err(|e| e.to_string())?;
        if raw.len() < 13 {
            return Err("sealed secret too short".into());
        }
        let (nonce, ct) = raw.split_at(12);
        let nonce: [u8; 12] = nonce.try_into().map_err(|_| "bad nonce")?;
        let pt = self.cipher.decrypt(&nonce.into(), ct).map_err(|_| "decryption failed (wrong master key?)")?;
        String::from_utf8(pt).map_err(|e| e.to_string())
    }
}

/// Last 4 characters, for display.
pub fn last4(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let b = SecretBox::ephemeral();
        let s = b.encrypt("sk-test-1234");
        assert!(!s.contains("sk-test"));
        assert_eq!(b.decrypt(&s).unwrap(), "sk-test-1234");
        assert!(SecretBox::ephemeral().decrypt(&s).is_err());
        assert_eq!(last4("sk-test-1234"), "1234");
    }
}
