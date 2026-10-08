use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A blake3 content address, written `blake3:<64 lowercase hex>`.
///
/// Invariant: always 32 bytes. Equal content has equal digests; the digest is never computed
/// in this crate.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest {
    bytes: [u8; 32],
}

/// Why a string is not a valid [`Digest`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DigestError {
    /// The algorithm prefix is missing or not `blake3`.
    #[error("digest must start with \"blake3:\"")]
    UnsupportedAlgorithm,
    /// The hex part is not 64 characters.
    #[error("digest must have 64 hex characters, found {0}")]
    InvalidLength(usize),
    /// The hex part contains a character other than `0-9a-f`.
    #[error("digest contains invalid character {0:?}")]
    InvalidCharacter(char),
}

impl Digest {
    const PREFIX: &'static str = "blake3:";
    const HEX: &'static [u8; 16] = b"0123456789abcdef";

    /// A digest from a blake3 hash output.
    #[must_use]
    pub const fn from_blake3(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    /// The raw 32 bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    fn nibble(c: char) -> Result<u8, DigestError> {
        match c {
            '0'..='9' => Ok(c as u8 - b'0'),
            'a'..='f' => Ok(c as u8 - b'a' + 10),
            _ => Err(DigestError::InvalidCharacter(c)),
        }
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::PREFIX)?;
        for byte in self.bytes {
            let high = Self::HEX[usize::from(byte >> 4)];
            let low = Self::HEX[usize::from(byte & 0x0f)];
            write!(f, "{}{}", char::from(high), char::from(low))?;
        }
        Ok(())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Digest {
    type Err = DigestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix(Self::PREFIX)
            .ok_or(DigestError::UnsupportedAlgorithm)?;
        let chars: Vec<char> = hex.chars().collect();
        if chars.len() != 64 {
            return Err(DigestError::InvalidLength(chars.len()));
        }
        let mut bytes = [0u8; 32];
        let (pairs, _) = chars.as_chunks::<2>();
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            *byte = (Self::nibble(*high)? << 4) | Self::nibble(*low)?;
        }
        Ok(Self { bytes })
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn rejects_malformed_input() {
        let hex = "ab".repeat(32);
        assert_eq!(
            format!("sha256:{hex}").parse::<Digest>(),
            Err(DigestError::UnsupportedAlgorithm)
        );
        assert_eq!(
            "blake3:abc".parse::<Digest>(),
            Err(DigestError::InvalidLength(3))
        );
        assert_eq!(
            format!("blake3:{}", "AB".repeat(32)).parse::<Digest>(),
            Err(DigestError::InvalidCharacter('A'))
        );
    }

    proptest! {
        #[test]
        fn any_bytes_round_trip(bytes in any::<[u8; 32]>()) {
            let digest = Digest::from_blake3(bytes);
            prop_assert_eq!(digest.to_string().parse::<Digest>(), Ok(digest));
        }
    }
}
