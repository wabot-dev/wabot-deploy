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
        let repository = crate::platform::images::Reference::parse(&self.reference)
            .map(|reference| reference.repository)
            .unwrap_or_else(|| self.reference.clone());
        format!("{repository}@{}", self.digest)
    }
}
