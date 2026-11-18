//! A release: one image digest, and where it came from.

use serde::Serialize;

/// Where a release came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The registry received it.
    Push,
    /// Somebody pointed the service at an image.
    Manual,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Push => "push",
            Source::Manual => "manual",
        }
    }

    pub(crate) fn parse(text: &str) -> Self {
        match text {
            "push" => Source::Push,
            _ => Source::Manual,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::Push => "Pushed",
            Source::Manual => "Set by hand",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Release {
    pub id: String,
    pub service_id: String,
    pub reference: String,
    pub digest: String,
    pub source: Source,
    pub created_at: i64,
    /// When this one was last deployed, if it ever was.
    pub deployed_at: Option<i64>,
}

impl Release {
    /// The tag this was pushed as: `latest`, `v2`, `2026-08-19`.
    ///
    /// What tells two releases of one service apart at a glance — the
    /// repository is the same on every one of them, and the digest is
    /// eight hex characters nobody can hold in their head. Falls back to
    /// the whole reference when there is no tag to take, which is a
    /// digest-pinned reference and reads as itself.
    pub fn tag(&self) -> &str {
        // From the last colon, and only if it is after the last slash: a
        // registry may carry a port, and `node:5000/app` has a colon that
        // is not a tag's.
        match self.reference.rsplit_once(':') {
            Some((before, tag)) if !before.ends_with('/') && !tag.contains('/') => tag,
            _ => &self.reference,
        }
    }

    /// The digest as somebody would read it: `sha256:1a2b3c4d`.
    ///
    /// Enough to tell two builds apart at a glance, and the full value
    /// is one hover away in the title attribute.
    pub fn short_digest(&self) -> String {
        match self.digest.split_once(':') {
            Some((algorithm, hex)) => format!("{algorithm}:{}", &hex[..hex.len().min(12)]),
            None => self.digest.clone(),
        }
    }

    /// What to run: the repository at this exact digest.
    ///
    /// Not the tag it arrived under — that is a label on a moving
    /// target, and the point of a release is that it does not move.
    pub fn pinned(&self) -> String {
        let repository =
            crate::domain::values::image_reference::ImageReference::parse(&self.reference)
                .ok()
                .and_then(|r| r.parts())
                .map(|reference| reference.repository)
                .unwrap_or_else(|| self.reference.clone());
        format!("{repository}@{}", self.digest)
    }

    /// The release a service running `image` is on, out of its releases.
    ///
    /// Only one deployed under that very reference. A service somebody
    /// pointed at another image since is running that image, and pinning
    /// it back to a release of the old one would undo the change without
    /// a word.
    pub fn current<'a>(releases: &'a [Release], image: &str) -> Option<&'a Release> {
        releases
            .iter()
            .find(|release| release.deployed_at.is_some() && release.reference == image)
    }

    /// The releases grouped by the image they are, in the order given.
    pub fn versions(releases: &[Release]) -> Vec<Version> {
        let mut versions: Vec<Version> = Vec::new();
        for release in releases {
            match versions
                .iter_mut()
                .find(|version| version.digest() == release.digest)
            {
                Some(version) => version.releases.push(release.clone()),
                None => versions.push(Version {
                    releases: vec![release.clone()],
                }),
            }
        }
        versions
    }
}

/// One image of a service, under however many tags it arrived.
///
/// A build pushed as `latest` and as its run's own name is two releases and
/// one image, and the list of versions showed it twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    releases: Vec<Release>,
}

impl Version {
    pub fn releases(&self) -> &[Release] {
        &self.releases
    }

    pub fn digest(&self) -> &str {
        &self.releases[0].digest
    }

    pub fn short_digest(&self) -> String {
        self.releases[0].short_digest()
    }

    pub fn tags(&self) -> Vec<&str> {
        let mut tags: Vec<&str> = Vec::new();
        for release in &self.releases {
            if !tags.contains(&release.tag()) {
                tags.push(release.tag());
            }
        }
        tags
    }

    pub fn source(&self) -> Source {
        self.releases[0].source
    }

    pub fn running(&self) -> bool {
        self.releases
            .iter()
            .any(|release| release.deployed_at.is_some())
    }

    /// The release its Deploy button sends: the one under the reference the
    /// service watches, so the next restart finds it current.
    pub fn to_deploy(&self, image: &str) -> &Release {
        self.releases
            .iter()
            .find(|release| release.reference == image)
            .unwrap_or(&self.releases[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(id: &str, reference: &str, deployed_at: Option<i64>) -> Release {
        Release {
            id: id.into(),
            service_id: "svc".into(),
            reference: reference.into(),
            digest: format!("sha256:{id}"),
            source: Source::Push,
            created_at: 0,
            deployed_at,
        }
    }

    #[test]
    fn the_current_release_is_the_deployed_one_not_the_newest() {
        let releases = [
            release("new", "hub.example/app:latest", None),
            release("old", "hub.example/app:latest", Some(1)),
        ];
        let current = Release::current(&releases, "hub.example/app:latest").expect("one");
        assert_eq!(current.id, "old");
        assert_eq!(current.pinned(), "hub.example/app@sha256:old");
    }

    #[test]
    fn a_service_pointed_at_another_image_has_no_current_release() {
        let releases = [release("old", "hub.example/app:latest", Some(1))];
        assert!(Release::current(&releases, "docker.io/library/nginx:alpine").is_none());
        assert!(Release::current(&releases, "hub.example/app:v2").is_none());
    }

    /// Jorge's CI pushes each build of `nubo-testing` as `latest` and as the
    /// tag it ran for, `testing-20261117-2`: one digest, two rows, and only
    /// the row under the deployed tag could say Running (2026-11-17).
    #[test]
    fn one_image_under_two_tags_is_one_version() {
        let latest = release("b", "hub.example/app:latest", Some(2));
        let named = release("b", "hub.example/app:testing-2", None);
        let older = release("a", "hub.example/app:testing-1", None);

        let versions = Release::versions(&[named, latest, older]);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].digest(), "sha256:b");
        assert_eq!(versions[0].tags(), ["testing-2", "latest"]);
        assert!(versions[0].running(), "running under either tag");
        assert_eq!(versions[1].tags(), ["testing-1"]);
        assert!(!versions[1].running());
    }

    #[test]
    fn a_version_deploys_under_the_tag_the_service_watches() {
        let versions = Release::versions(&[
            release("b", "hub.example/app:testing-2", None),
            release("b", "hub.example/app:latest", None),
        ]);
        assert_eq!(
            versions[0].to_deploy("hub.example/app:latest").reference,
            "hub.example/app:latest"
        );
        assert_eq!(
            versions[0].to_deploy("hub.example/app:v9").reference,
            "hub.example/app:testing-2",
            "the newest when it watches none of them"
        );
    }

    #[test]
    fn an_image_already_pinned_has_no_current_release_to_pin_it_to() {
        let releases = [release("old", "hub.example/app:latest", Some(1))];
        assert!(Release::current(&releases, "hub.example/app@sha256:old").is_none());
    }
}
