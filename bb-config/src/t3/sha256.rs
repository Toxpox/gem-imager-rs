//! Typed SHA-256 digest for T3 catalog integrity fields.

use std::fmt;

/// A parsed SHA-256 digest.
///
/// the catalog validation rules requires every catalog hash to be exactly 64 hex characters and to be
/// converted to `[u8; 32]` while parsing. Keeping the digest as a typed value (rather than a
/// `String`) makes it impossible to compare a truncated or malformed hex string downstream, and
/// makes the archive/extracted digests non-interchangeable at the type level.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256([u8; Sha256::LEN]);

/// Why a hex string could not be parsed into a [`Sha256`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sha256ParseError {
    /// Digest was not exactly [`Sha256::HEX_LEN`] bytes long.
    Length {
        /// Length actually seen, in bytes.
        actual: usize,
    },
    /// Digest contained a byte outside `[0-9a-fA-F]`.
    NotHex,
}

impl Sha256 {
    /// Length of the raw digest in bytes.
    pub const LEN: usize = 32;
    /// Length of the hex encoding in bytes.
    pub const HEX_LEN: usize = 64;

    /// Parse a 64-character hex string.
    ///
    /// Rejects any other length up front so a short digest can never be zero-extended.
    pub fn parse(hex: &str) -> Result<Self, Sha256ParseError> {
        if hex.len() != Self::HEX_LEN {
            return Err(Sha256ParseError::Length { actual: hex.len() });
        }

        let mut raw = [0u8; Self::LEN];
        const_hex::decode_to_slice(hex, &mut raw).map_err(|_| Sha256ParseError::NotHex)?;
        Ok(Self(raw))
    }

    /// Wrap an already-computed digest, e.g. one produced by a hasher during download.
    pub const fn from_bytes(raw: [u8; Self::LEN]) -> Self {
        Self(raw)
    }

    /// Raw digest bytes, for comparison against a hasher result.
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }

    /// Lowercase hex encoding, for logs and persistence.
    pub fn to_hex(&self) -> String {
        const_hex::encode(self.0)
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256({})", self.to_hex())
    }
}

impl fmt::Display for Sha256ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { actual } => {
                write!(
                    f,
                    "expected {} hex characters, got {actual}",
                    Sha256::HEX_LEN
                )
            }
            Self::NotHex => f.write_str("contains non-hexadecimal characters"),
        }
    }
}

impl std::error::Error for Sha256ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "9c991802d2ceff5a80cfd3e822f9cd2f9730cee59759714ba7530c81a824c92d";

    #[test]
    fn parses_valid_lowercase_digest() {
        let hash = Sha256::parse(VALID).expect("valid digest");
        assert_eq!(hash.to_hex(), VALID);
    }

    #[test]
    fn parses_uppercase_and_normalises_to_lowercase() {
        let hash = Sha256::parse(&VALID.to_uppercase()).expect("valid digest");
        assert_eq!(hash.to_hex(), VALID);
    }

    #[test]
    fn rejects_short_digest_instead_of_zero_extending() {
        let err = Sha256::parse(&VALID[..63]).unwrap_err();
        assert_eq!(err, Sha256ParseError::Length { actual: 63 });
    }

    #[test]
    fn rejects_long_digest() {
        let err = Sha256::parse(&format!("{VALID}0")).unwrap_err();
        assert_eq!(err, Sha256ParseError::Length { actual: 65 });
    }

    #[test]
    fn rejects_non_hex_characters() {
        let mut bad = VALID.to_owned();
        bad.replace_range(0..1, "z");
        assert_eq!(Sha256::parse(&bad).unwrap_err(), Sha256ParseError::NotHex);
    }

    #[test]
    fn rejects_empty_digest() {
        assert_eq!(
            Sha256::parse("").unwrap_err(),
            Sha256ParseError::Length { actual: 0 }
        );
    }

    #[test]
    fn round_trips_through_raw_bytes() {
        let hash = Sha256::parse(VALID).expect("valid digest");
        assert_eq!(Sha256::from_bytes(*hash.as_bytes()), hash);
    }
}
