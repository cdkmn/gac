use aes_gcm::{
    aead::{Aead, Generate, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use argon2::{
    password_hash::rand_core::{OsRng, RngCore},
    Argon2,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

const NONCE_SIZE: usize = 12;
const KEY_SIZE: usize = 32;
const SALT_SIZE: usize = 16;

/// Argon2 parameters for key derivation.
/// These are tuned for interactive use (low latency, reasonable memory).
const ARGON2_M_COST: u32 = 65536; // 64 MiB
const ARGON2_T_COST: u32 = 3; // 3 iterations
const ARGON2_P_COST: u32 = 1; // 1 lane

/// Derive a 32-byte key from a password using Argon2id with a random salt.
/// Returns the salt concatenated with the derived key (both base64-encoded).
fn derive_key(password: &str, salt: Option<&[u8]>) -> Result<(Vec<u8>, [u8; KEY_SIZE])> {
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(KEY_SIZE))
            .map_err(|e| anyhow::anyhow!("invalid argon2 params: {e}"))?,
    );

    let salt = match salt {
        Some(s) => s.to_vec(),
        None => {
            let mut salt = vec![0u8; SALT_SIZE];
            OsRng.fill_bytes(&mut salt);
            salt
        }
    };

    let mut key = [0u8; KEY_SIZE];
    argon2
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(|e| anyhow::anyhow!("argon2 key derivation failed: {e}"))?;

    Ok((salt, key))
}

/// Get the encryption password from environment variable or use a default.
/// In a real application, you'd want to use a securer key derivation
/// (e.g., Argon2, PBKDF2) and store the password securely.
fn get_password() -> String {
    std::env::var("GAC_ENCRYPTION_KEY")
        .unwrap_or_else(|_| "gac-default-key-change-in-production".to_string())
}

/// Encrypt a plaintext string using AES-256-GCM with Argon2id key derivation.
/// Returns a base64-encoded string containing: salt (16 bytes) || nonce (12 bytes) || ciphertext+tag.
/// The salt is stored with the ciphertext so decryption can derive the same key.
pub fn encrypt(plaintext: &str) -> Result<String> {
    let password = get_password();
    let (salt, key) = derive_key(&password, None)?;
    let cipher = Aes256Gcm::new(&key.into());
    let nonce = Nonce::generate();
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .context("failed to encrypt")?;

    // Combine salt || nonce || ciphertext (which includes the auth tag)
    let mut combined = Vec::with_capacity(SALT_SIZE + NONCE_SIZE + ciphertext.len());
    combined.extend_from_slice(&salt);
    combined.extend_from_slice(&nonce);
    combined.extend_from_slice(&ciphertext);

    Ok(BASE64.encode(combined))
}

/// Decrypt a base64-encoded string that was encrypted with `encrypt`.
/// Format: salt (16 bytes) || nonce (12 bytes) || ciphertext+tag
/// The salt is used to re-derive the decryption key.
pub fn decrypt(encrypted: &str) -> Result<String> {
    let password = get_password();

    let combined = BASE64
        .decode(encrypted)
        .context("failed to decode base64")?;

    if combined.len() < SALT_SIZE + NONCE_SIZE {
        anyhow::bail!("ciphertext too short");
    }

    let (salt, rest) = combined.split_at(SALT_SIZE);
    let (nonce_bytes, ciphertext) = rest.split_at(NONCE_SIZE);
    let nonce = Nonce::try_from(nonce_bytes).context("failed to slice to nonce")?;

    // Re-derive key using the stored salt
    let (_, key) = derive_key(&password, Some(salt))?;
    let cipher = Aes256Gcm::new(&key.into());

    let plaintext = cipher
        .decrypt(&nonce, ciphertext)
        .context("failed to decrypt - wrong key or corrupted data")?;

    String::from_utf8(plaintext).context("decrypted data is not valid UTF-8")
}
