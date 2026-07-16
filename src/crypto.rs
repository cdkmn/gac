use aes_gcm::{
    aead::{Aead, Generate, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use sha2::{Digest, Sha256};

const NONCE_SIZE: usize = 12;
const KEY_SIZE: usize = 32;

/// Derive a 32-byte key from a password using SHA-256.
fn derive_key(password: &str) -> [u8; KEY_SIZE] {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; KEY_SIZE];
    key.copy_from_slice(&result);
    key
}

/// Get the encryption password from environment variable or use a default.
/// In a real application, you'd want to use a more secure key derivation
/// (e.g., Argon2, PBKDF2) and store the password securely.
fn get_password() -> String {
    std::env::var("GAC_ENCRYPTION_KEY")
        .unwrap_or_else(|_| "gac-default-key-change-in-production".to_string())
}

/// Encrypt a plaintext string using AES-256-GCM.
/// Returns a base64-encoded string containing nonce + ciphertext + tag.
pub fn encrypt(plaintext: &str) -> Result<String> {
    let password = get_password();
    let key = derive_key(&password);
    let cipher = Aes256Gcm::new(&key.into());
    let nonce = Nonce::generate();
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .context("failed to encrypt")?;

    // Combine nonce + ciphertext (which includes the auth tag)
    let mut combined = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    combined.extend_from_slice(&nonce);
    combined.extend_from_slice(&ciphertext);

    Ok(BASE64.encode(combined))
}

/// Decrypt a base64-encoded string that was encrypted with `encrypt`.
pub fn decrypt(encrypted: &str) -> Result<String> {
    let password = get_password();
    let key = derive_key(&password);
    let cipher = Aes256Gcm::new(&key.into());

    let combined = BASE64
        .decode(encrypted)
        .context("failed to decode base64")?;

    if combined.len() < NONCE_SIZE {
        anyhow::bail!("ciphertext too short");
    }

    let (nonce_bytes, ciphertext) = combined.split_at(NONCE_SIZE);
    let nonce = Nonce::try_from(nonce_bytes).context("failed to slice to nonce")?;

    let plaintext = cipher
        .decrypt(&nonce, ciphertext)
        .context("failed to decrypt - wrong key or corrupted data")?;

    String::from_utf8(plaintext).context("decrypted data is not valid UTF-8")
}
