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
            // A hash of the assets, from `build.rs`. It used to be the
            // crate version, which does not move when the assets do —
            // and these are served `must-revalidate`, so every browser
            // that had seen an earlier build of the same version asked,
            // was told 304, and kept the stale file. A feature worked on
            // the server and did nothing in the page, with no error
            // anywhere. `env!` because `concat!` needs a literal.
            etag: concat!("\"", env!("WABOT_ASSET_HASH"), "-", $path, "\""),
        }
    };
}

/// The faces this page actually renders in, and nothing else.
///
/// ## Why preload at all
///
/// The stylesheet is render-blocking, so there is no flash of unstyled
/// *layout* — but the browser only discovers `@font-face` after it has
/// parsed the CSS, which is one round trip too late. Text paints in the
/// system fallback and then swaps to Geist, and that swap is the
/// flicker. A preload starts the font fetch in parallel with the
/// stylesheet instead of after it, so the real face is usually there
/// before first paint.
///
/// ## Why only five of the six
///
/// A preload the page never uses is a wasted high-priority request
/// competing with the ones that matter, and Chrome says so in the
/// console. `GeistMono-Medium` is shipped — the stylesheet asks for it,
/// and something later will use it — but nothing on this page does.
///
/// | face | what renders in it |
/// | --- | --- |
/// | Regular (400) | body text, the tagline, the notes |
/// | Medium (500) | badges, card labels, `dl.kv dt` |
/// | SemiBold (600) | feature names, h2–h6 |
/// | Bold (700) | the `h1` |
/// | Mono Regular | `dl.kv dd`, `pre code` |
pub const PRELOAD_FONTS: &[&str] = &[
    "fonts/Geist-Regular.woff2",
    "fonts/Geist-Medium.woff2",
    "fonts/Geist-SemiBold.woff2",
    "fonts/Geist-Bold.woff2",
    "fonts/GeistMono-Regular.woff2",
];

pub const ASSETS: EmbeddedAssets = &[
    asset!("wabot.css", "text/css; charset=utf-8"),
    asset!("console.js", "text/javascript; charset=utf-8"),
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

    /// A preload for a path that is not served costs a request, warns
    /// in the console, and does not prevent the flicker it was added
    /// for — worse than not preloading at all.
    #[test]
    fn every_preloaded_font_is_shipped() {
        let shipped: Vec<&str> = ASSETS.iter().map(|asset| asset.path).collect();
        for font in PRELOAD_FONTS {
            assert!(
                shipped.contains(font),
                "{font} is preloaded but not compiled in"
            );
        }
    }

    /// And a preload has to be a font, or `as="font"` is a lie the
    /// browser acts on.
    #[test]
    fn preloads_are_all_woff2() {
        for font in PRELOAD_FONTS {
            let asset = ASSETS
                .iter()
                .find(|asset| asset.path == *font)
                .expect("shipped");
            assert_eq!(asset.content_type, "font/woff2", "{font}");
        }
    }

    /// The one face this page does not render in stays unpreloaded.
    /// If a later page uses it, this list grows — and this assertion is
    /// what makes that a deliberate edit rather than an oversight.
    #[test]
    fn the_unused_face_is_not_preloaded() {
        assert!(
            !PRELOAD_FONTS.contains(&"fonts/GeistMono-Medium.woff2"),
            "nothing on the home page renders in mono medium"
        );
    }

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

    /// The tag has to be a hash of the assets, not of anything that
    /// can stay still while they change. Four deploys of 0.1.5 shipped
    /// four different `console.js` files under one version-derived tag,
    /// and every browser that had seen an earlier one kept it.
    #[test]
    fn the_tag_does_not_come_from_the_version() {
        let tag = ASSETS
            .iter()
            .find(|asset| asset.path == "console.js")
            .expect("shipped")
            .etag;
        assert!(
            !tag.contains(env!("CARGO_PKG_VERSION")),
            "the version cannot be what identifies an asset: {tag}"
        );
        assert!(tag.contains(env!("WABOT_ASSET_HASH")), "{tag}");
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

    /// A stale asset served from cache is a confusing bug — it looks
    /// like a feature that does nothing rather than like an error — so
    /// the tag has to move when the *asset* does, which is not the same
    /// as when the version does.
    #[test]
    fn etags_are_distinct_and_versioned() {
        let mut tags: Vec<&str> = ASSETS.iter().map(|asset| asset.etag).collect();
        let total = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), total, "two assets share an ETag");

        // The hash, not the version: a release that changes an asset
        // without changing the version has to serve a new tag, or the
        // browser keeps what it had.
        assert!(ASSETS
            .iter()
            .all(|asset| asset.etag.contains(env!("WABOT_ASSET_HASH"))));
    }
}
