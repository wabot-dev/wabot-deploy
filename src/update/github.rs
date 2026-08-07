//! The release catalogue, read from GitHub's public API.
//!
//! No token: the repository is public, and an updater that needs a
//! credential to see what it could update to is one more thing to
//! expire. Unauthenticated requests are rate-limited per IP — sixty an
//! hour — which is why the catalogue is cached rather than fetched on
//! every page view. See [`super::Catalogue`].

use serde::Deserialize;

use super::http;

/// Where releases come from. A constant rather than configuration: an
/// updater that will download and *execute* what it finds must not
/// take that address from a file somebody can edit, and a fork that
/// wants its own releases is a fork that can change this line.
pub const REPOSITORY: &str = "wabot-dev/wabot-deploy";

/// What the published binaries are called. The workflow names them
/// `wabot-deploy-<version>-<arch>-linux`, and this is the half that
/// identifies the machine.
fn asset_suffix() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("-x86_64-linux"),
        "aarch64" => Some("-aarch64-linux"),
        _ => None,
    }
}

pub type ReleaseResult<T> = Result<T, ReleaseError>;

#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error(transparent)]
    Http(#[from] http::HttpError),
    #[error("GitHub answered something this cannot read: {0}")]
    Malformed(String),
    #[error("no release is tagged {0}")]
    Unknown(String),
    #[error("this machine is {0}, which nothing is built for yet")]
    Unsupported(&'static str),
}

/// A version, as far as ordering needs to care.
///
/// Hand-parsed rather than a dependency: the tags this reads are the
/// ones its own release workflow writes, and they are `vX.Y.Z`.
/// Anything else — a release candidate, a date — fails to parse and is
/// therefore never offered, which is the right way for an updater to
/// be wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('v');
        let mut parts = text.split('.');
        let mut next = || parts.next()?.parse::<u32>().ok();
        let (major, minor, patch) = (next()?, next()?, next()?);
        // A fourth component means this is not the shape of version
        // being compared, and guessing is worse than declining.
        match parts.next() {
            Some(_) => None,
            None => Some(Self {
                major,
                minor,
                patch,
            }),
        }
    }

    /// What this binary is.
    pub fn current() -> Option<Self> {
        Self::parse(crate::api::VERSION)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One published release, with the parts an update needs.
#[derive(Debug, Clone)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    /// The release title, or the tag when GitHub has no other name.
    pub name: String,
    /// The notes, as Markdown. Rendered by [`super::notes`].
    pub notes: String,
    /// ISO-8601, as GitHub sends it.
    pub published_at: String,
    pub prerelease: bool,
    pub html_url: String,
    /// The binary for *this* machine, and the checksum beside it.
    /// Absent when the release published no build for this
    /// architecture — which is a release this node cannot install.
    pub binary: Option<Asset>,
    pub checksum: Option<Asset>,
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

impl Release {
    /// Can this node install it? Both parts have to be published: the
    /// binary, and the checksum that says the binary arrived intact.
    pub fn installable(&self) -> bool {
        self.binary.is_some() && self.checksum.is_some()
    }
}

/// Every published release, newest version first.
///
/// Drafts are skipped — they have no assets and are not public — and
/// so is anything whose tag is not `vX.Y.Z`, since an update is a
/// comparison and an unorderable version cannot be compared.
pub async fn releases() -> ReleaseResult<Vec<Release>> {
    let suffix = asset_suffix().ok_or(ReleaseError::Unsupported(std::env::consts::ARCH))?;

    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=30");
    let body = http::get_text(&url, "application/vnd.github+json").await?;
    let raw: Vec<RawRelease> =
        serde_json::from_str(&body).map_err(|error| ReleaseError::Malformed(error.to_string()))?;

    let mut releases: Vec<Release> = raw
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| release.into_release(suffix))
        .collect();
    releases.sort_by_key(|release| std::cmp::Reverse(release.version));
    Ok(releases)
}

/// The newest release this node could install, if it is newer than
/// what is running.
///
/// Pre-releases are never offered: somebody who wants one can pick it
/// from the list, which is a different act than accepting whatever is
/// newest.
pub fn newest_upgrade(releases: &[Release], current: Option<Version>) -> Option<&Release> {
    let current = current?;
    releases
        .iter()
        .filter(|release| !release.prerelease && release.installable())
        .find(|release| release.version > current)
}

pub fn find<'a>(releases: &'a [Release], tag: &str) -> Option<&'a Release> {
    let wanted = Version::parse(tag);
    releases
        .iter()
        .find(|release| release.tag == tag || (wanted.is_some() && Some(release.version) == wanted))
}

// ---------- what GitHub actually sends -------------------------------

#[derive(Debug, Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Debug, Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

impl RawRelease {
    fn into_release(self, suffix: &str) -> Option<Release> {
        let version = Version::parse(&self.tag_name)?;

        let asset = |wanted: &dyn Fn(&str) -> bool| -> Option<Asset> {
            self.assets
                .iter()
                .find(|asset| wanted(&asset.name))
                .map(|asset| Asset {
                    name: asset.name.clone(),
                    url: asset.browser_download_url.clone(),
                    size: asset.size,
                })
        };

        let binary = asset(&|name: &str| name.ends_with(suffix));
        let checksum = asset(&|name: &str| name.ends_with(&format!("{suffix}.sha256")));

        Some(Release {
            name: self
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| self.tag_name.clone()),
            tag: self.tag_name,
            version,
            notes: self.body.unwrap_or_default(),
            published_at: self.published_at.unwrap_or_default(),
            prerelease: self.prerelease,
            html_url: self.html_url,
            binary,
            checksum,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = r###"[
      {
        "tag_name": "v0.2.0",
        "name": "0.2.0",
        "body": "## What changed\n- one thing",
        "published_at": "2026-08-07T10:00:00Z",
        "draft": false,
        "prerelease": false,
        "html_url": "https://github.com/wabot-dev/wabot-deploy/releases/tag/v0.2.0",
        "assets": [
          {"name": "wabot-deploy-0.2.0-x86_64-linux", "browser_download_url": "https://example/bin", "size": 21},
          {"name": "wabot-deploy-0.2.0-x86_64-linux.sha256", "browser_download_url": "https://example/sum", "size": 8}
        ]
      },
      {
        "tag_name": "v0.3.0-rc1",
        "draft": false,
        "prerelease": true,
        "assets": []
      },
      {
        "tag_name": "v0.1.0",
        "draft": false,
        "prerelease": false,
        "assets": [
          {"name": "wabot-deploy-0.1.0-x86_64-linux", "browser_download_url": "https://example/old", "size": 20},
          {"name": "wabot-deploy-0.1.0-x86_64-linux.sha256", "browser_download_url": "https://example/oldsum", "size": 8}
        ]
      },
      {
        "tag_name": "v9.9.9",
        "draft": true,
        "prerelease": false,
        "assets": []
      }
    ]"###;

    fn parsed() -> Vec<Release> {
        let raw: Vec<RawRelease> = serde_json::from_str(PAYLOAD).expect("payload");
        let mut releases: Vec<Release> = raw
            .into_iter()
            .filter(|release| !release.draft)
            .filter_map(|release| release.into_release("-x86_64-linux"))
            .collect();
        releases.sort_by_key(|release| std::cmp::Reverse(release.version));
        releases
    }

    #[test]
    fn versions_order_by_number_not_by_text() {
        let two = Version::parse("v0.2.0").expect("parse");
        let ten = Version::parse("0.10.0").expect("parse");
        assert!(ten > two, "0.10.0 is newer than 0.2.0");
        assert_eq!(two.to_string(), "0.2.0");
    }

    /// Anything not `vX.Y.Z` is declined rather than guessed at — an
    /// updater that mis-orders versions installs the wrong one.
    #[test]
    fn an_unorderable_tag_is_no_version() {
        for tag in ["v0.3.0-rc1", "nightly", "v1.2", "v1.2.3.4", "v1.2.x"] {
            assert_eq!(Version::parse(tag), None, "{tag}");
        }
    }

    #[test]
    fn a_draft_is_not_a_release_and_a_prerelease_has_no_version() {
        let releases = parsed();
        let tags: Vec<&str> = releases.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(tags, vec!["v0.2.0", "v0.1.0"]);
    }

    #[test]
    fn the_assets_for_this_machine_are_picked_out() {
        let releases = parsed();
        let newest = &releases[0];

        assert!(newest.installable());
        assert_eq!(
            newest.binary.as_ref().expect("binary").url,
            "https://example/bin"
        );
        assert_eq!(
            newest.checksum.as_ref().expect("checksum").url,
            "https://example/sum"
        );
        // The checksum must not be mistaken for the binary: its name
        // also ends with the architecture, plus four characters.
        assert!(!newest
            .binary
            .as_ref()
            .expect("binary")
            .name
            .ends_with(".sha256"));
    }

    #[test]
    fn only_something_newer_is_an_upgrade() {
        let releases = parsed();

        let from_one = newest_upgrade(&releases, Version::parse("0.1.0"));
        assert_eq!(from_one.map(|r| r.tag.as_str()), Some("v0.2.0"));

        assert!(newest_upgrade(&releases, Version::parse("0.2.0")).is_none());
        assert!(newest_upgrade(&releases, Version::parse("1.0.0")).is_none());
    }

    #[test]
    fn a_release_can_be_found_by_tag_or_by_version() {
        let releases = parsed();
        assert_eq!(
            find(&releases, "v0.1.0").map(|r| r.tag.as_str()),
            Some("v0.1.0")
        );
        assert_eq!(
            find(&releases, "0.1.0").map(|r| r.tag.as_str()),
            Some("v0.1.0")
        );
        assert!(find(&releases, "v5.0.0").is_none());
    }
}
