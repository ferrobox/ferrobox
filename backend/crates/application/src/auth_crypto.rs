//! Hashing of passwords (Argon2) and of API token secrets (SHA-256).

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use password_hash::rand_core::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Visible prefix of every token secret issued by `FerroBox`. Lets you
/// recognize them at a glance (for example, in
/// `~/.cargo/credentials.toml`) without leaking the full secret.
pub const TOKEN_PREFIX_TAG: &str = "fb_";

/// Number of characters of the non-sensitive prefix shown in listings
/// (includes the `fb_` tag).
const VISIBLE_PREFIX_LEN: usize = 11;

/// Reasons hashing a password can fail.
#[derive(Debug, Error)]
pub enum PasswordHashError {
    /// The hashing library returned an internal error.
    #[error("failed to hash password")]
    HashingFailed,
}

/// Computes the Argon2id hash of a plaintext password.
///
/// # Errors
///
/// Returns [`PasswordHashError::HashingFailed`] if the library cannot
/// generate the salt or the hash.
pub fn hash_password(password: &str) -> Result<String, PasswordHashError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordHashError::HashingFailed)
}

/// Verifies a plaintext password against a stored Argon2 hash.
#[must_use]
pub fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(password_hash) else {
        return false;
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Generates a new API token secret (`fb_` + 32 bytes in hex) and its
/// non-sensitive prefix for listings.
#[must_use]
pub fn generate_api_token_secret() -> (String, String) {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let secret = format!("{TOKEN_PREFIX_TAG}{}", hex::encode(bytes));
    let prefix = secret.chars().take(VISIBLE_PREFIX_LEN).collect();
    (secret, prefix)
}

/// Computes the SHA-256 (hex) hash of a token secret. Used both when
/// issuing and when authenticating, so the plaintext secret is never
/// persisted.
#[must_use]
pub fn hash_api_token_secret(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trips_through_argon2() {
        let hash = hash_password("s3cret").unwrap();

        assert!(verify_password("s3cret", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn generated_token_secret_has_expected_shape() {
        let (secret, prefix) = generate_api_token_secret();

        assert!(secret.starts_with(TOKEN_PREFIX_TAG));
        assert_eq!(prefix, &secret[..VISIBLE_PREFIX_LEN]);
        assert_eq!(hash_api_token_secret(&secret).len(), 64);
    }
}
