use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// A type with a TypeID prefix, such as `"sbx"` for sandboxes.
///
/// Invariant: `PREFIX` is 1 to 63 lowercase ASCII letters or underscores, starting and ending
/// with a letter.
pub trait Prefixed {
    /// The TypeID prefix of every [`Id`] of this type.
    const PREFIX: &'static str;
}

/// A typed identifier following the TypeID specification: `<prefix>_<26 base32 chars>`.
///
/// Wraps any 128-bit UUID; IDs are generated as UUIDv7 by the `IdGenerator` port, so they sort
/// by creation time. Display, parsing and serde use the same textual form.
pub struct Id<T: Prefixed> {
    raw: Uuid,
    _marker: PhantomData<fn() -> T>,
}

/// Why a string is not a valid [`Id`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// The prefix breaks the TypeID prefix rules.
    #[error("invalid TypeID prefix {0:?}")]
    InvalidPrefix(String),
    /// The prefix is valid but belongs to another type.
    #[error("expected prefix {expected:?}, found {found:?}")]
    WrongPrefix {
        /// The prefix of the requested type.
        expected: &'static str,
        /// The prefix found in the input.
        found: String,
    },
    /// The suffix is not exactly 26 characters.
    #[error("TypeID suffix must be 26 characters, found {0}")]
    InvalidLength(usize),
    /// The suffix contains a character outside the TypeID alphabet.
    #[error("TypeID suffix contains invalid character {0:?}")]
    InvalidCharacter(char),
    /// The suffix encodes more than 128 bits.
    #[error("TypeID suffix encodes more than 128 bits")]
    Overflow,
}

impl<T: Prefixed> Id<T> {
    /// Wraps a UUID. Only the `IdGenerator` port and tests should call this.
    #[must_use]
    pub const fn from_uuid(raw: Uuid) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    /// The underlying UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.raw
    }
}

impl<T: Prefixed> From<Id<T>> for Uuid {
    fn from(id: Id<T>) -> Self {
        id.raw
    }
}

impl<T: Prefixed> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !T::PREFIX.is_empty() {
            write!(f, "{}_", T::PREFIX)?;
        }
        Suffix::from(self.raw).fmt(f)
    }
}

impl<T: Prefixed> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<T: Prefixed> FromStr for Id<T> {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts = TypeIdParts::try_from(s)?;
        if parts.prefix != T::PREFIX {
            return Err(IdError::WrongPrefix {
                expected: T::PREFIX,
                found: parts.prefix.to_owned(),
            });
        }
        Ok(Self::from_uuid(parts.uuid))
    }
}

impl<T: Prefixed> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Prefixed> Copy for Id<T> {}

impl<T: Prefixed> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<T: Prefixed> Eq for Id<T> {}

impl<T: Prefixed> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<T: Prefixed> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Prefixed> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<T: Prefixed> Serialize for Id<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de, T: Prefixed> Deserialize<'de> for Id<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// A TypeID split into its validated prefix and decoded UUID.
struct TypeIdParts<'a> {
    prefix: &'a str,
    uuid: Uuid,
}

impl<'a> TryFrom<&'a str> for TypeIdParts<'a> {
    type Error = IdError;

    fn try_from(s: &'a str) -> Result<Self, Self::Error> {
        let (prefix, suffix) = match s.rsplit_once('_') {
            Some((prefix, suffix)) => {
                if !Self::is_valid_prefix(prefix) {
                    return Err(IdError::InvalidPrefix(prefix.to_owned()));
                }
                (prefix, suffix)
            }
            None => ("", s),
        };
        let uuid = Suffix::try_from(suffix)?.into();
        Ok(Self { prefix, uuid })
    }
}

impl TypeIdParts<'_> {
    /// Whether `prefix` is a non-empty TypeID prefix: `[a-z]([a-z_]{0,61}[a-z])?`.
    fn is_valid_prefix(prefix: &str) -> bool {
        let bytes = prefix.as_bytes();
        let (Some(first), Some(last)) = (bytes.first(), bytes.last()) else {
            return false;
        };
        bytes.len() <= 63
            && first.is_ascii_lowercase()
            && last.is_ascii_lowercase()
            && bytes.iter().all(|b| b.is_ascii_lowercase() || *b == b'_')
    }
}

/// The 26-character TypeID suffix: 128 bits in base32, with two leading zero bits.
struct Suffix(u128);

impl Suffix {
    const ALPHABET: &'static [u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    const LEN: usize = 26;

    fn digit(c: char) -> Option<u128> {
        let byte = u8::try_from(c).ok()?;
        Self::ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .map(|index| index as u128)
    }
}

impl From<Uuid> for Suffix {
    fn from(uuid: Uuid) -> Self {
        Self(uuid.as_u128())
    }
}

impl From<Suffix> for Uuid {
    fn from(suffix: Suffix) -> Self {
        Uuid::from_u128(suffix.0)
    }
}

impl TryFrom<&str> for Suffix {
    type Error = IdError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let length = s.chars().count();
        if length != Self::LEN {
            return Err(IdError::InvalidLength(length));
        }
        let mut value: u128 = 0;
        for (index, c) in s.chars().enumerate() {
            let digit = Self::digit(c).ok_or(IdError::InvalidCharacter(c))?;
            // The first digit carries only the top 3 of 130 bits; anything above 7 overflows.
            if index == 0 && digit > 7 {
                return Err(IdError::Overflow);
            }
            value = (value << 5) | digit;
        }
        Ok(Self(value))
    }
}

impl fmt::Display for Suffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buffer = [0u8; Self::LEN];
        for (index, slot) in buffer.iter_mut().enumerate() {
            let shift = 5 * (Self::LEN - 1 - index);
            let digit = (self.0 >> shift) & 0x1f;
            *slot = Self::ALPHABET[digit as usize];
        }
        // The buffer holds only ASCII alphabet bytes.
        f.write_str(std::str::from_utf8(&buffer).map_err(|_| fmt::Error)?)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    enum Bare {}
    impl Prefixed for Bare {
        const PREFIX: &'static str = "";
    }

    enum Pre {}
    impl Prefixed for Pre {
        const PREFIX: &'static str = "prefix";
    }

    enum PreFix {}
    impl Prefixed for PreFix {
        const PREFIX: &'static str = "pre_fix";
    }

    fn uuid(s: &str) -> Uuid {
        Uuid::parse_str(s).expect("valid test uuid")
    }

    /// `spec/valid.yml` from the TypeID specification.
    #[test]
    fn valid_spec_vectors() {
        let bare = [
            (
                "00000000000000000000000000",
                "00000000-0000-0000-0000-000000000000",
            ),
            (
                "00000000000000000000000001",
                "00000000-0000-0000-0000-000000000001",
            ),
            (
                "0000000000000000000000000a",
                "00000000-0000-0000-0000-00000000000a",
            ),
            (
                "0000000000000000000000000g",
                "00000000-0000-0000-0000-000000000010",
            ),
            (
                "00000000000000000000000010",
                "00000000-0000-0000-0000-000000000020",
            ),
            (
                "7zzzzzzzzzzzzzzzzzzzzzzzzz",
                "ffffffff-ffff-ffff-ffff-ffffffffffff",
            ),
        ];
        for (text, raw) in bare {
            let id: Id<Bare> = text.parse().expect(text);
            assert_eq!(id.as_uuid(), uuid(raw), "{text}");
            assert_eq!(id.to_string(), text);
        }
        let prefixed = [
            (
                "prefix_0123456789abcdefghjkmnpqrs",
                "0110c853-1d09-52d8-d73e-1194e95b5f19",
            ),
            (
                "prefix_01h455vb4pex5vsknk084sn02q",
                "01890a5d-ac96-774b-bcce-b302099a8057",
            ),
        ];
        for (text, raw) in prefixed {
            let id: Id<Pre> = text.parse().expect(text);
            assert_eq!(id.as_uuid(), uuid(raw), "{text}");
            assert_eq!(id.to_string(), text);
        }
        let text = "pre_fix_00000000000000000000000000";
        let id: Id<PreFix> = text.parse().expect(text);
        assert_eq!(id.as_uuid(), Uuid::nil());
        assert_eq!(id.to_string(), text);
    }

    /// `spec/invalid.yml` from the TypeID specification.
    #[test]
    fn invalid_spec_vectors() {
        let invalid = [
            "PREFIX_00000000000000000000000000",
            "12345_00000000000000000000000000",
            "pre.fix_00000000000000000000000000",
            "préfix_00000000000000000000000000",
            "  prefix_00000000000000000000000000",
            "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijkl_00000000000000000000000000",
            "_00000000000000000000000000",
            "_",
            "prefix_1234567890123456789012345",
            "prefix_123456789012345678901234567",
            "prefix_1234567890123456789012345 ",
            "prefix_0123456789ABCDEFGHJKMNPQRS",
            "prefix_123456789-123456789-123456",
            "prefix_ooooooiiiiiiuuuuuuulllllll",
            "prefix_i23456789ol23456789oi23456",
            "prefix_123456789-0123456789-0123456",
            "prefix_8zzzzzzzzzzzzzzzzzzzzzzzzz",
            "_prefix_00000000000000000000000000",
            "prefix__00000000000000000000000000",
            "",
            "prefix_",
        ];
        for text in invalid {
            assert!(
                TypeIdParts::try_from(text).is_err(),
                "{text:?} must be invalid"
            );
        }
    }

    #[test]
    fn each_failure_has_its_own_variant() {
        let wrong_prefix = "other_00000000000000000000000000".parse::<Id<Pre>>();
        assert!(matches!(wrong_prefix, Err(IdError::WrongPrefix { .. })));
        let bad_prefix = "Pre_00000000000000000000000000".parse::<Id<Pre>>();
        assert!(matches!(bad_prefix, Err(IdError::InvalidPrefix(_))));
        let short = "prefix_0000".parse::<Id<Pre>>();
        assert_eq!(short, Err(IdError::InvalidLength(4)));
        let bad_char = "prefix_0000000000000000000000000u".parse::<Id<Pre>>();
        assert_eq!(bad_char, Err(IdError::InvalidCharacter('u')));
        let overflow = "prefix_8zzzzzzzzzzzzzzzzzzzzzzzzz".parse::<Id<Pre>>();
        assert_eq!(overflow, Err(IdError::Overflow));
    }

    #[test]
    fn serializes_as_its_display_form() {
        let id = Id::<Pre>::from_uuid(uuid("01890a5d-ac96-774b-bcce-b302099a8057"));
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"prefix_01h455vb4pex5vsknk084sn02q\"");
        let back: Id<Pre> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
    }

    proptest! {
        #[test]
        fn any_uuid_round_trips(raw in any::<u128>()) {
            let id = Id::<Pre>::from_uuid(Uuid::from_u128(raw));
            prop_assert_eq!(id.to_string().parse::<Id<Pre>>(), Ok(id));
        }
    }
}
