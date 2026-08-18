//! Reading an image reference.
//!
//! Everything about matching a push to a service goes through here, so
//! the parsing is in one tested place rather than repeated wherever
//! somebody needed a tag.

/// The parts of `host/namespace/name:tag`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Everything before the tag: `docker.io/library/nginx`.
    pub repository: String,
    /// `latest` when the reference did not say.
    pub tag: String,
}

impl Reference {
    /// Split a reference into its repository and tag.
    ///
    /// The tag is what follows the **last** colon, and only when that
    /// colon comes after the last slash — otherwise `host:5000/name`
    /// reads as the repository `host` at tag `5000/name`, which is how
    /// a registry running on a port breaks everything downstream.
    ///
    /// A digest reference — `name@sha256:…` — has no tag. It names one
    /// exact image, so there is nothing to watch for updates: `None`
    /// says so rather than inventing `latest`.
    pub fn parse(reference: &str) -> Option<Self> {
        let reference = reference.trim();
        if reference.is_empty() || reference.contains('@') {
            return None;
        }

        let last_slash = reference.rfind('/').map(|at| at + 1).unwrap_or(0);
        match reference[last_slash..].rfind(':') {
            Some(colon) => {
                let at = last_slash + colon;
                Some(Self {
                    repository: reference[..at].to_string(),
                    tag: reference[at + 1..].to_string(),
                })
            }
            None => Some(Self {
                repository: reference.to_string(),
                // What every registry client assumes when a reference
                // carries no tag, so assuming anything else would make
                // this node the odd one out.
                tag: "latest".to_string(),
            }),
        }
    }

    /// The name without its registry host: `library/nginx` out of
    /// `docker.io/library/nginx`.
    ///
    /// What a push arrives as — the client strips the host it dialled
    /// — so this is how a stored reference is compared to one.
    pub fn name(&self) -> &str {
        match self.repository.split_once('/') {
            // A first segment with a dot or a port is a hostname. One
            // without is a namespace: `library/nginx` is not hosted on
            // a machine called `library`.
            Some((head, rest)) if head.contains('.') || head.contains(':') => rest,
            _ => &self.repository,
        }
    }
}

/// The tag a service watches for new images.
///
/// Its own setting when it has one, and otherwise the tag its image
/// reference already names — so a service created against `:latest`
/// watches `latest` without anybody configuring it.
pub fn tracked_tag(image: &str, track_tag: Option<&str>) -> Option<String> {
    let _ = image;
    match track_tag {
        Some(tag) if !tag.trim().is_empty() => Some(tag.trim().to_string()),
        // **`latest`, not the tag the service's image happens to carry.**
        //
        // It used to be the image's, and the settings form has always
        // shown `latest` as this field's placeholder — so the interface
        // promised one thing and the code did another. A service running
        // `docker.io/library/nginx:alpine` with an empty field was
        // watching `alpine`: a tag borrowed from somebody else's
        // repository, for a registry the image is not even in.
        //
        // Which of the two to keep was decided by the placeholder. It is
        // the part an operator reads, it said `latest`, and every
        // reference the console's own push example produces carries
        // `latest` too. Making the code agree with what the form already
        // promised is a smaller surprise than changing the promise.
        //
        // A service pinned to `…/app:v2` in this registry now needs `v2`
        // typed here to record releases from a `v2` push — and the badge
        // on the service page says which tag is watched, so that is a
        // visible state rather than a silent one. Reported by Jorge.
        _ => Some("latest".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_reference_splits_into_repository_and_tag() {
        let reference = Reference::parse("docker.io/library/nginx:alpine").expect("parsed");
        assert_eq!(reference.repository, "docker.io/library/nginx");
        assert_eq!(reference.tag, "alpine");
    }

    /// What every registry client assumes. Assuming anything else here
    /// would make this node the odd one out.
    #[test]
    fn no_tag_means_latest() {
        assert_eq!(
            Reference::parse("docker.io/library/nginx")
                .expect("parsed")
                .tag,
            "latest"
        );
    }

    /// The case a naive split on the first colon gets wrong, and the
    /// reason this function exists: a registry on a port.
    #[test]
    fn a_registry_port_is_not_a_tag() {
        let reference = Reference::parse("node.example:5000/team/api:v2").expect("parsed");
        assert_eq!(reference.repository, "node.example:5000/team/api");
        assert_eq!(reference.tag, "v2");

        let untagged = Reference::parse("node.example:5000/team/api").expect("parsed");
        assert_eq!(untagged.repository, "node.example:5000/team/api");
        assert_eq!(untagged.tag, "latest");
    }

    /// A digest names one exact image. There is nothing to watch, and
    /// pretending it is tagged `latest` would make a push to `latest`
    /// look like an update to it.
    #[test]
    fn a_digest_reference_has_no_tag() {
        assert_eq!(
            Reference::parse("docker.io/library/nginx@sha256:abc123"),
            None
        );
    }

    /// A push arrives with the host stripped, so this is what a stored
    /// reference has to be compared as.
    #[test]
    fn the_name_drops_the_registry_host() {
        let hosted = Reference::parse("node.example/team/api:v1").expect("parsed");
        assert_eq!(hosted.name(), "team/api");

        let ported = Reference::parse("node.example:5000/team/api:v1").expect("parsed");
        assert_eq!(ported.name(), "team/api");
    }

    /// `library/nginx` is not hosted on a machine called `library`.
    #[test]
    fn a_namespace_is_not_a_host() {
        let namespaced = Reference::parse("library/nginx:alpine").expect("parsed");
        assert_eq!(namespaced.name(), "library/nginx");

        let bare = Reference::parse("nginx:alpine").expect("parsed");
        assert_eq!(bare.name(), "nginx");
    }

    /// Nothing chosen means `latest`, whatever tag the image carries.
    ///
    /// This asserted the opposite — that a service watches the tag it
    /// was created with — and the settings form has always shown
    /// `latest` as this field's placeholder. So the interface promised
    /// one thing and the code did another, and a service running
    /// `docker.io/library/nginx:alpine` announced `watching :alpine`: a
    /// tag borrowed from a repository this node does not host.
    ///
    /// The placeholder decided it. It is the part somebody reads, and
    /// every reference the console's own push example produces carries
    /// `latest`. Reported by Jorge.
    #[test]
    fn nothing_chosen_means_latest() {
        assert_eq!(
            tracked_tag("docker.io/library/nginx:alpine", None).as_deref(),
            Some("latest")
        );
        assert_eq!(
            tracked_tag("node.example/team/api", None).as_deref(),
            Some("latest")
        );
    }

    #[test]
    fn its_own_setting_wins() {
        assert_eq!(
            tracked_tag("node.example/team/api:v1", Some("production")).as_deref(),
            Some("production")
        );
        // Empty is not a setting, so it falls to the default rather than
        // to the image's `v1`. A service pinned to a tag in this
        // registry has to say so here, and the badge on its page shows
        // which tag is watched so that this is visible rather than
        // silent.
        assert_eq!(
            tracked_tag("node.example/team/api:v1", Some("  ")).as_deref(),
            Some("latest")
        );
    }

    #[test]
    fn nothing_is_parsed_out_of_nothing() {
        assert_eq!(Reference::parse(""), None);
        assert_eq!(Reference::parse("   "), None);
    }
}
