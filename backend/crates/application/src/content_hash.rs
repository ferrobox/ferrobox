use ferrobox_domain::checksum::Sha256Checksum;
use sha2::{Digest, Sha256};

/// Calcula el checksum SHA-256 de un contenido binario.
///
/// # Panics
///
/// En la práctica, nunca entra en pánico: SHA-256 siempre produce
/// exactamente 32 bytes, cuya codificación hexadecimal son siempre 64
/// caracteres válidos -- la condición que [`Sha256Checksum::parse`]
/// podría rechazar nunca ocurre con esta entrada.
pub(crate) fn sha256_checksum(content: &[u8]) -> Sha256Checksum {
    Sha256Checksum::parse(format!("{:x}", Sha256::digest(content)))
        .expect("a hex-encoded SHA-256 digest is always a valid Sha256Checksum")
}
