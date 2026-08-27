use std::fmt;

use crate::errors::{PlatformError, PlatformResult};

/// A reference that names something containerd can resolve.
///
/// Immutable and checked once: an `ImageReference` in hand has already
/// been through the rules below, so no caller asks again. It also knows
/// how to take itself apart — the repository, the tag and the registry
/// host used to be three parsers in two modules, and two of them shipped
/// the same bug an hour apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReference(String);

/// A reference taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parts {
    /// Everything before the tag: `docker.io/library/nginx`.
    pub repository: String,
    /// `latest` when the reference did not say.
    pub tag: String,
}

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

    /// The repository and the tag.
    ///
    /// The tag is what follows the **last** colon, and only when that
    /// colon comes after the last slash — otherwise `host:5000/name`
    /// reads as the repository `host` at tag `5000/name`, which is how a
    /// registry running on a port breaks everything downstream.
    ///
    /// A digest reference — `name@sha256:…` — has no tag. It names one
    /// exact image, so there is nothing to watch for updates: `None`
    /// says so rather than inventing `latest`.
    pub fn parts(&self) -> Option<Parts> {
        let reference = self.0.trim();
        if reference.is_empty() || reference.contains('@') {
            return None;
        }

        let last_slash = reference.rfind('/').map(|at| at + 1).unwrap_or(0);
        Some(match reference[last_slash..].rfind(':') {
            Some(colon) => {
                let at = last_slash + colon;
                Parts {
                    repository: reference[..at].to_string(),
                    tag: reference[at + 1..].to_string(),
                }
            }
            None => Parts {
                repository: reference.to_string(),
                // What every registry client assumes when a reference
                // carries no tag, so assuming anything else would make
                // this node the odd one out.
                tag: "latest".to_string(),
            },
        })
    }

    /// The registry this reference names, when it names one.
    ///
    /// Without the first condition `alpine:3.23` reads as a registry
    /// called `alpine` on port `3.23`, because a tag's colon looks
    /// exactly like a port's. That shipped for an hour in two copies of
    /// this rule, which is why there is one and it is here.
    ///
    /// `library/alpine` is not a host called `library` either — that is
    /// the second condition doing its job.
    pub fn host(&self) -> Option<String> {
        let (first, _) = self.0.split_once('/')?;
        let looks_like_a_host = first.contains('.') || first.contains(':') || first == "localhost";
        looks_like_a_host.then(|| first.to_string())
    }
}

impl Parts {
    /// The name without its registry host: `library/nginx` out of
    /// `docker.io/library/nginx`.
    ///
    /// What a push arrives as — the client strips the host it dialled —
    /// so this is how a stored reference is compared to one.
    pub fn name(&self) -> &str {
        match self.repository.split_once('/') {
            // A first segment with a dot or a port is a hostname. One
            // without is a namespace: `library/nginx` is not hosted on a
            // machine called `library`.
            Some((head, rest)) if head.contains('.') || head.contains(':') => rest,
            _ => &self.repository,
        }
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

    fn parse(reference: &str) -> ImageReference {
        ImageReference::parse(reference).expect("parsed")
    }

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
        assert!(parse("   ").is_empty());
    }

    #[test]
    fn it_is_trimmed_once_and_kept() {
        assert_eq!(parse("  nginx:alpine ").as_str(), "nginx:alpine");
    }

    /// A registry on a port: the colon before the last slash is not a
    /// tag's, and reading it as one breaks everything downstream.
    #[test]
    fn a_port_in_the_host_is_not_a_tag() {
        let parts = parse("host:5000/name").parts().expect("has a tag");
        assert_eq!(parts.repository, "host:5000/name");
        assert_eq!(parts.tag, "latest");
    }

    #[test]
    fn no_tag_means_latest() {
        assert_eq!(parse("nginx").parts().expect("parts").tag, "latest");
    }

    /// A digest names one exact image, so there is nothing to watch.
    #[test]
    fn a_digest_has_no_tag_to_watch() {
        assert!(parse("nginx@sha256:abc").parts().is_none());
    }

    #[test]
    fn the_name_drops_the_registry_host() {
        assert_eq!(
            parse("docker.io/library/nginx:alpine")
                .parts()
                .expect("parts")
                .name(),
            "library/nginx"
        );
        assert_eq!(
            parse("library/nginx").parts().expect("parts").name(),
            "library/nginx"
        );
    }

    /// `library/alpine` is not a host called `library`. Getting this
    /// wrong would send a node's credential to Docker Hub.
    #[test]
    fn a_namespace_is_not_a_registry() {
        assert_eq!(
            parse("hub.example.com/proj/app:v1").host().as_deref(),
            Some("hub.example.com")
        );
        assert_eq!(
            parse("localhost:5000/proj/app").host().as_deref(),
            Some("localhost:5000")
        );
        assert_eq!(
            parse("localhost/proj/app").host().as_deref(),
            Some("localhost")
        );

        assert_eq!(parse("library/alpine:3.23").host(), None);
        assert_eq!(parse("alpine").host(), None);
        assert_eq!(parse("").host(), None);

        // The one that got away: a tag's colon looks exactly like a
        // port's, so a reference with no path at all read as a registry
        // called `alpine` on port `3.23`.
        assert_eq!(parse("alpine:3.23").host(), None);
        assert_eq!(parse("ubuntu:24.04").host(), None);
    }

    #[test]
    fn a_plain_reference_splits_into_repository_and_tag() {
        let reference = ImageReference::parse("docker.io/library/nginx:alpine")
            .expect("valid")
            .parts()
            .expect("parsed");
        assert_eq!(reference.repository, "docker.io/library/nginx");
        assert_eq!(reference.tag, "alpine");
    }
    /// What every registry client assumes. Assuming anything else here
    /// The case a naive split on the first colon gets wrong, and the
    /// A digest names one exact image. There is nothing to watch, and
    /// A push arrives with the host stripped, so this is what a stored
    /// `library/nginx` is not hosted on a machine called `library`.
    #[test]
    fn nothing_is_parsed_out_of_nothing() {
        assert_eq!(ImageReference::parse("").expect("valid").parts(), None);
        assert_eq!(ImageReference::parse("   ").expect("valid").parts(), None);
    }
}
