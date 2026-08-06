//! The chrome every console page shares, and the CSS it needs.
//!
//! One place, because a header that drifts between pages is how a
//! console starts feeling like three consoles.

use hypertext::prelude::*;
use hypertext::Raw;

use crate::accounts::Account;

use super::assets;

/// Declare the head every console page needs.
///
/// Called by each view rather than by a layout function, because the
/// framework assembles the document and a view's only way in is the
/// render scope.
pub fn head(title_text: &str) {
    use wabot::ui::hypertext::{link, style, title};

    title(format!("{title_text} · wabot-deploy"));

    for font in assets::PRELOAD_FONTS {
        link([
            ("rel", "preload"),
            ("as", "font"),
            ("type", "font/woff2"),
            ("crossorigin", "anonymous"),
            ("href", &format!("{}/{font}", assets::MOUNT)),
        ]);
    }
    style(format!("{}/wabot.css", assets::MOUNT));
    link([
        ("rel", "icon"),
        ("type", "image/png"),
        ("href", &format!("{}/favicon.png", assets::MOUNT)),
    ]);
}

/// The bar across the top of a signed-in page.
pub fn header(account: &Account) -> impl Renderable + '_ {
    rsx! {
        <header class="topbar">
            <a class="brand" href="/">
                <img
                    src=(format!("{}/wabot-logo.png", assets::MOUNT))
                    alt="Wabot" width="24" height="24">
                <span>("wabot-deploy")</span>
            </a>
            <div class="row">
                <span class="who">(&account.username)</span>
                // A form, not a link: signing out changes state, and a
                // GET that changes state is one a prefetcher can fire.
                <form method="post" action="/sign-out">
                    <button class="btn btn-ghost btn-sm" type="submit">("Sign out")</button>
                </form>
            </div>
        </header>
    }
}

/// The error strip a form shows when the node refused it.
pub fn error_note(message: &str) -> impl Renderable + '_ {
    rsx! {
        <p class="form-error"><strong>("Error: ")</strong>(message)</p>
    }
}

/// Page-specific layout. Colour, type and radii come from the design
/// system's tokens; this is arrangement, which is the console's own
/// business. No borders, no shadows, no hover states — separation is
/// background contrast.
pub const CSS: &str = r#"
.shell {
  max-width: 62rem;
  margin: 0 auto;
  padding: var(--sp-8) var(--sp-6) var(--sp-12);
  display: flex;
  flex-direction: column;
  gap: var(--sp-8);
}
.narrow { max-width: 30rem; }

.topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-4);
  padding: var(--sp-4) var(--sp-6);
  background: rgb(var(--c-bg-sunken));
}
.brand {
  display: inline-flex;
  align-items: center;
  gap: var(--sp-3);
  font-weight: 600;
  color: rgb(var(--c-fg));
  text-decoration: none;
}
.who { color: rgb(var(--c-fg-muted)); font-size: var(--fs-sm); }

.mark { display: flex; align-items: center; gap: var(--sp-5); }
.mark h1 { font-size: var(--fs-3xl); margin: 0; letter-spacing: -0.03em; }
.tagline { color: rgb(var(--c-fg-muted)); margin: 0; }

.note {
  margin: var(--sp-4) 0 0;
  color: rgb(var(--c-fg-muted));
  font-size: var(--fs-sm);
  max-width: 46rem;
}
.form-error {
  background: rgb(var(--c-danger-bg));
  color: rgb(var(--c-danger-fg));
  padding: var(--sp-3) var(--sp-4);
  border-radius: var(--r-md);
  font-size: var(--fs-sm);
  margin: 0 0 var(--sp-4);
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
  gap: var(--sp-4);
}
.tile { display: block; color: inherit; text-decoration: none; }
.tile-name { margin: 0 0 var(--sp-2); font-weight: 600; }
.tile-detail { margin: 0; color: rgb(var(--c-fg-muted)); font-size: var(--fs-sm); }

/* The reason a service is not running, under the service it is about.
   Full width because a containerd error is a paragraph, not a word. */
.failure {
  color: rgb(var(--c-danger-fg));
  background: rgb(var(--c-danger-bg));
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  white-space: pre-wrap;
  word-break: break-word;
}

.empty {
  padding: var(--sp-10) 0;
  color: rgb(var(--c-fg-muted));
  text-align: center;
}
.empty p { margin: 0 0 var(--sp-4); }

/* The design system gives a field the raised tone — the same tone a
   card has — because it expects fields to sit on the canvas. Inside a
   card they vanish: white on white, and no borders to save them. Dark
   mode already resolves exactly this collision by moving fields to the
   contrast tone; light mode needs it whenever the surface underneath
   is raised. */
.card input,
.card textarea,
.card select {
  background: rgb(var(--c-bg-contrast));
}

/* A label belongs to the field under it. An even stack gap makes the
   two read as unrelated rows, so the field is pulled back up against
   its label and the space falls between pairs instead. */
.stack label + input,
.stack label + textarea,
.stack label + select {
  margin-top: calc(var(--sp-1) - var(--sp-4));
}

.slug-preview {
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-faint));
}
.field-hint {
  margin: calc(var(--sp-1) * -1) 0 var(--sp-4);
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-muted));
}
.actions { display: flex; gap: var(--sp-3); align-items: center; margin-top: var(--sp-5); }

.foot {
  display: flex;
  justify-content: space-between;
  gap: var(--sp-4);
  flex-wrap: wrap;
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-muted));
}
@media (max-width: 40rem) {
  .shell { padding: var(--sp-6) var(--sp-4) var(--sp-10); gap: var(--sp-6); }
  .mark h1 { font-size: var(--fs-2xl); }
}
"#;

/// The page CSS as a `<style>` element.
pub fn style_tag() -> impl Renderable {
    // XSS SAFETY: a `const` in this file, never a value from a request.
    rsx! { <style>(Raw::dangerously_create(CSS))</style> }
}
