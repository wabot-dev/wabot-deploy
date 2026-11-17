use std::fmt;

use serde::Serialize;

pub const LENGTH: &str = "a bucket name is 3 to 63 characters long";
pub const CHARACTERS: &str = "a bucket name is lowercase letters, digits and hyphens, \
     and starts and ends with a letter or a digit";
pub const PUNYCODE: &str = "a bucket name cannot start with xn--, which a browser would show \
     as another name";
pub const RESERVED: &str = "a bucket name cannot end with -s3alias, which S3 keeps for itself";

/// A bucket's name: S3's rule, less the dot.
///
/// A dotted name cannot be one label under the store's name, and
/// virtual-hosted addressing will need it to be.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct BucketName(String);

impl BucketName {
    pub fn parse(typed: &str) -> Result<Self, &'static str> {
        let name = typed.trim().to_ascii_lowercase();
        if !(3..=63).contains(&name.len()) {
            return Err(LENGTH);
        }
        let allowed = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
        if !name.bytes().all(|byte| allowed(byte) || byte == b'-')
            || !name.bytes().next().is_some_and(allowed)
            || !name.bytes().last().is_some_and(allowed)
        {
            return Err(CHARACTERS);
        }
        if name.starts_with("xn--") {
            return Err(PUNYCODE);
        }
        if name.ends_with("-s3alias") {
            return Err(RESERVED);
        }
        Ok(Self(name))
    }

    /// A name as a row holds it, which `parse` accepted when it was written.
    /// Read back without the rule, so a rule made stricter later cannot hide
    /// a bucket that still holds objects.
    pub fn stored(text: &str) -> Self {
        Self(text.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BucketName {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_somebody_types_becomes_the_name_a_client_will_use() {
        for typed in ["uploads", "  Uploads ", "UPLOADS"] {
            assert_eq!(
                BucketName::parse(typed).expect("a name").as_str(),
                "uploads"
            );
        }
        assert!(BucketName::parse("user-files-2026").is_ok());
        assert!(BucketName::parse("abc").is_ok());
        assert!(BucketName::parse(&"a".repeat(63)).is_ok());
    }

    #[test]
    fn a_name_too_short_or_too_long_says_how_long_one_is() {
        assert_eq!(BucketName::parse("ab"), Err(LENGTH));
        assert_eq!(BucketName::parse("   "), Err(LENGTH));
        assert_eq!(BucketName::parse(&"a".repeat(64)), Err(LENGTH));
    }

    #[test]
    fn a_dot_is_refused_although_s3_allows_one() {
        assert_eq!(BucketName::parse("my.bucket"), Err(CHARACTERS));
    }

    #[test]
    fn a_name_starts_and_ends_with_a_letter_or_a_digit() {
        for typed in ["-uploads", "uploads-", "up_loads", "up loads", "subidas-ñ"] {
            assert_eq!(BucketName::parse(typed), Err(CHARACTERS), "{typed}");
        }
    }

    #[test]
    fn a_name_that_would_read_as_another_is_refused() {
        assert_eq!(BucketName::parse("xn--bcher-kva"), Err(PUNYCODE));
        assert_eq!(BucketName::parse("files-s3alias"), Err(RESERVED));
    }
}
