//! Hashing de contraseñas (Argon2) y de secretos de tokens de API
//! (SHA-256).

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use password_hash::rand_core::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Prefijo visible de todos los secretos de token emitidos por
/// `FerroBox`. Permite reconocerlos a simple vista (por ejemplo, en
/// `~/.cargo/credentials.toml`) sin filtrar el secreto completo.
pub const TOKEN_PREFIX_TAG: &str = "fb_";

/// Número de caracteres del prefijo no sensible que se muestra en
/// listados (incluye el tag `fb_`).
const VISIBLE_PREFIX_LEN: usize = 11;

/// Motivos por los que el hashing de una contraseña puede fallar.
#[derive(Debug, Error)]
pub enum PasswordHashError {
    /// La librería de hashing devolvió un error interno.
    #[error("failed to hash password")]
    HashingFailed,
}

/// Calcula el hash Argon2id de una contraseña en claro.
///
/// # Errors
///
/// Devuelve [`PasswordHashError::HashingFailed`] si la librería no
/// puede generar la sal o el hash.
pub fn hash_password(password: &str) -> Result<String, PasswordHashError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordHashError::HashingFailed)
}

/// Verifica una contraseña en claro contra un hash Argon2 almacenado.
#[must_use]
pub fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(password_hash) else {
        return false;
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Genera un secreto de token de API nuevo (`fb_` + 32 bytes en hex)
/// y su prefijo no sensible para listados.
#[must_use]
pub fn generate_api_token_secret() -> (String, String) {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let secret = format!("{TOKEN_PREFIX_TAG}{}", hex::encode(bytes));
    let prefix = secret.chars().take(VISIBLE_PREFIX_LEN).collect();
    (secret, prefix)
}

/// Calcula el hash SHA-256 (hex) de un secreto de token. Se usa tanto
/// al emitir como al autenticar, de modo que el secreto en claro nunca
/// se persiste.
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
