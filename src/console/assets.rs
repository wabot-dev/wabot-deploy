//! The design system, compiled into the binary.
//!
//! Vendored rather than linked from `design.wabot.dev`, and that is the
//! whole point of the product restated: a node that fetched its
//! stylesheet from the internet would render unstyled on the private
//! network, the air-gapped rack, or the afternoon someone's CDN is
//! down. The console has to work where the node works.
//!
//! ~330 KB, almost all of it Geist. Against an 8 MB binary that is a
//! fair price for looking like Wabot rather than like whatever the
//! browser fell back to.

use wabot::ui::{EmbeddedAsset, EmbeddedAssets};

/// Where the assets are mounted. `_wabot` is the framework's own
/// prefix for machinery, which keeps it clear of any hostname a user
/// might later route.
pub const MOUNT: &str = "/_wabot/assets";

macro_rules! asset {
    ($path:literal, $type:literal) => {
        EmbeddedAsset {
            path: $path,
            bytes: include_bytes!(concat!("../../assets/", $path)),
            content_type: $type,
            // The build hash would be better, but `EmbeddedAsset`
            // wants a `&'static str` and there is no build script
            // here. The version is enough: assets only change when the
            // binary does.
            etag: concat!("\"", env!("CARGO_PKG_VERSION"), "-", $path, "\""),
        }
    };
}

pub const ASSETS: EmbeddedAssets = &[
    asset!("wabot.css", "text/css; charset=utf-8"),
    asset!("wabot-logo.png", "image/png"),
    asset!("favicon.png", "image/png"),
    asset!("fonts/Geist-Regular.woff2", "font/woff2"),
    asset!("fonts/Geist-Medium.woff2", "font/woff2"),
    asset!("fonts/Geist-SemiBold.woff2", "font/woff2"),
    asset!("fonts/Geist-Bold.woff2", "font/woff2"),
    asset!("fonts/GeistMono-Regular.woff2", "font/woff2"),
    asset!("fonts/GeistMono-Medium.woff2", "font/woff2"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The stylesheet asks for `fonts/Geist-Regular.woff2` and friends
    /// by relative path. A missing one is a 404 the browser papers
    /// over with a fallback face — the page still renders, slightly
    /// wrong, and nobody notices until someone looks closely.
    #[test]
    fn every_font_the_stylesheet_asks_for_is_shipped() {
        let css = std::str::from_utf8(
            ASSETS
                .iter()
                .find(|asset| asset.path == "wabot.css")
                .expect("the stylesheet is shipped")
                .bytes,
        )
        .expect("the stylesheet is utf-8");

        let shipped: Vec<&str> = ASSETS.iter().map(|asset| asset.path).collect();

        for line in css.lines() {
            let Some(start) = line.find("url('") else {
                continue;
            };
            let rest = &line[start + 5..];
            let Some(end) = rest.find('\'') else { continue };
            let referenced = &rest[..end];

            assert!(
                shipped.contains(&referenced),
                "the stylesheet asks for {referenced}, which is not compiled in"
            );
        }
    }

    #[test]
    fn nothing_is_empty() {
        for asset in ASSETS {
            assert!(
                !asset.bytes.is_empty(),
                "{} is empty — a failed download, probably",
                asset.path
            );
        }
    }

    /// A stale asset served from cache after an upgrade is a confusing
    /// bug, so the tag has to move when the binary does.
    #[test]
    fn etags_are_distinct_and_versioned() {
        let mut tags: Vec<&str> = ASSETS.iter().map(|asset| asset.etag).collect();
        let total = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), total, "two assets share an ETag");

        assert!(ASSETS
            .iter()
            .all(|asset| asset.etag.contains(env!("CARGO_PKG_VERSION"))));
    }
}
