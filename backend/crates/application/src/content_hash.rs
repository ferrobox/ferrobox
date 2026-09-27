use ferrobox_domain::checksum::Sha256Checksum;
use sha2::{Digest, Sha256};

/// Computes the SHA-256 checksum of binary content.
///
/// # Panics
///
/// In practice this never panics: SHA-256 always produces exactly 32
/// bytes, whose hexadecimal encoding is always 64 valid characters --
/// the condition that [`Sha256Checksum::parse`] could reject never
/// occurs with this input.
pub(crate) fn sha256_checksum(content: &[u8]) -> Sha256Checksum {
    Sha256Checksum::parse(format!("{:x}", Sha256::digest(content)))
        .expect("a hex-encoded SHA-256 digest is always a valid Sha256Checksum")
}
