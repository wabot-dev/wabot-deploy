use std::fmt;

use crate::platform::{PlatformError, PlatformResult};

/// A reference that names something containerd can resolve.
///
/// Immutable and checked once: a `ImageReference` in hand has already
/// been through the rules below, so no caller asks again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReference(String);

impl ImageReference {
    /// **Empty is a state, not a mistake.** A service can be created with
    /// no image and wait for the first push to its own repository —
    /// which is how somebody deploys their own application, and used to
    /// mean typing a reference for an image that did not exist yet. What
    /// refuses to act on it is the deploy path, which has something to
    /// say about it; this can only refuse the creation. Reported by
    /// Jorge.
    pub fn parse(typed: &str) -> PlatformResult<Self> {
        let image = typed.trim();
        if image.is_empty() {
            return Ok(Self(String::new()));
        }
        if image.contains("://") {
            return Err(PlatformError::Refused(format!(
                "an image reference is not a URL: {image:?} — try docker.io/library/nginx:alpine"
            )));
        }
        if image.split_whitespace().count() != 1 {
            return Err(PlatformError::Refused(
                "an image reference has no spaces in it".into(),
            ));
        }
        Ok(Self(image.to_string()))
    }

    /// A reference nothing has been pushed to yet.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ImageReference {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_not_an_image_reference() {
        let refused = ImageReference::parse("https://docker.io/nginx").expect_err("refused");
        assert!(refused.to_string().contains("not a URL"), "{refused}");
    }

    #[test]
    fn a_reference_has_no_spaces() {
        assert!(ImageReference::parse("nginx alpine").is_err());
    }

    /// A service waiting for its first push has no image, and that is a
    /// state rather than a mistake.
    #[test]
    fn nothing_pushed_yet_is_allowed() {
        let none = ImageReference::parse("   ").expect("allowed");
        assert!(none.is_empty());
    }

    #[test]
    fn it_is_trimmed_once_and_kept() {
        let reference = ImageReference::parse("  nginx:alpine ").expect("parsed");
        assert_eq!(reference.as_str(), "nginx:alpine");
    }
}
