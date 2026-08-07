//! The chrome every console page shares, and the CSS it needs.
//!
//! One place, because a header that drifts between pages is how a
//! console starts feeling like three consoles.

use hypertext::prelude::*;
use hypertext::Raw;

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

/// The error strip a form shows when the node refused it.
/// A moment, as how long ago it was.
///
/// Relative rather than absolute because every reader of this console
/// is looking at a machine, not a calendar: "4 minutes ago" answers
/// "did that just happen" without anybody working out what time zone
/// the node keeps. Absolute dates are for release notes, which come
/// with their own.
pub fn when(at_ms: i64) -> String {
    let seconds = (super::now_ms() - at_ms) / 1000;
    match seconds {
        // Clock skew, or a row written in the same tick.
        s if s < 0 => "just now".into(),
        s if s < 60 => "just now".into(),
        s if s < 3600 => plural(s / 60, "minute"),
        s if s < 86_400 => plural(s / 3600, "hour"),
        s => plural(s / 86_400, "day"),
    }
}

fn plural(count: i64, unit: &str) -> String {
    match count {
        1 => format!("1 {unit} ago"),
        many => format!("{many} {unit}s ago"),
    }
}

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

/* Release notes. Written by somebody else, arriving as Markdown, and
   rendered here as ordinary prose — narrow enough to read, with the
   bullets and code the notes actually use. */
.notes { max-width: 46rem; }
.notes h3 {
  font-size: var(--fs-md);
  margin: var(--sp-5) 0 var(--sp-2);
}
.notes h3:first-child { margin-top: 0; }
.notes p { margin: 0 0 var(--sp-3); }
.notes ul { margin: 0 0 var(--sp-3); padding-left: var(--sp-5); }
.notes li { margin: 0 0 var(--sp-1); }
.notes pre { margin: 0 0 var(--sp-3); overflow-x: auto; }

/* Back to where this page was reached from. Small, above the title,
   because the title is what somebody came to read. */
.crumb {
  margin: 0 0 var(--sp-2);
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-muted));
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

/* Selecting text inside a `<pre>` made it vanish. The design system
   sets one global `::selection` — brand at 22% over `--c-fg`, which is
   near-black — and a `<pre>` is near-black with light text. Black on a
   translucent tint over black is nothing at all.

   A dark surface needs its own: solid brand, white text. Upstream this
   wants to be part of the system's `pre` rules; here it compensates,
   like the button variants above. */
pre::selection,
pre ::selection {
  background: rgb(var(--c-brand));
  color: rgb(var(--c-n-0));
}

/* A checkbox and its words are one control, and they were touching:
   the box is `flex-shrink: 0` and the label had no gap, so the text
   started where the box ended. The label is also what makes the words
   clickable, which is most of the target. */
.check {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  cursor: pointer;
  font-weight: 500;
  margin-top: var(--sp-2);
}
.check input[type="checkbox"] { width: 1.05rem; }

/* Where a port is reachable, and whether its certificate has arrived.
   Two things in one cell, so the cell has to space them itself — and
   wrap rather than push the table sideways, because a hostname is as
   long as somebody's domain. */
.reach {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  flex-wrap: wrap;
}

/* The design system styles `button[type="submit"]` as the primary
   action, and that selector outranks `.btn-secondary`, `.btn-ghost`
   and `.btn-danger` — so every submit came out black whatever variant
   it asked for. On a console almost every action is a form, so almost
   every button was shouting. These restate the variants at a
   specificity that wins.

   The fix belongs upstream in the design system; this is the vendored
   copy, and editing it here would be lost on the next sync. */
button[type="submit"].btn-secondary {
  background: rgb(var(--c-action-soft));
  color: rgb(var(--c-fg));
}
button[type="submit"].btn-secondary:active { background: rgb(var(--c-action-soft) / 0.7); }

button[type="submit"].btn-ghost {
  background: transparent;
  color: rgb(var(--c-fg));
  padding-inline: var(--sp-3);
}
button[type="submit"].btn-ghost:active { color: rgb(var(--c-fg-muted)); }

button[type="submit"].btn-danger {
  background: rgb(var(--c-danger-fg));
  color: rgb(var(--c-n-0));
}

/* A destructive action that is not the danger zone: a row's Delete.
   Red text rather than a red slab — it has to read as destructive
   without competing with the page's one real action. */
.btn-ghost.destructive,
button[type="submit"].btn-ghost.destructive {
  color: rgb(var(--c-danger-fg));
}

/* The memory breakdown. One bar in parts, and a table under it whose
   swatches are the same colours — the bar says the proportions, the
   table says the numbers, and neither needs a legend. */
.meter {
  display: flex;
  height: 0.75rem;
  border-radius: var(--r-pill);
  overflow: hidden;
  background: rgb(var(--c-bg-contrast));
}
.meter-part { display: block; height: 100%; transition: width var(--motion-base); }
.meter-node       { background: rgb(var(--c-brand)); }
.meter-runtime    { background: rgb(var(--c-fg-muted)); }
.meter-containers { background: rgb(var(--c-success-fg)); }
.meter-rest       { background: rgb(var(--c-fg-faint) / 0.45); }

.mem { width: 100%; }
.mem td { vertical-align: baseline; }
.mem td:nth-child(2) { text-align: right; white-space: nowrap; }
.mem-key { display: flex; align-items: center; gap: var(--sp-2); white-space: nowrap; }
.swatch {
  width: 0.6rem;
  height: 0.6rem;
  border-radius: var(--r-sm);
  flex-shrink: 0;
}
.swatch.meter-node       { background: rgb(var(--c-brand)); }
.swatch.meter-runtime    { background: rgb(var(--c-fg-muted)); }
.swatch.meter-containers { background: rgb(var(--c-success-fg)); }
.swatch.meter-rest       { background: rgb(var(--c-fg-faint) / 0.45); }

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
///
/// The shell's layout comes with it: every page that renders inside
/// the frame needs both, and one tag means they can never arrive
/// half-applied.
pub fn style_tag() -> impl Renderable {
    // XSS SAFETY: two `const`s in this crate, never a value from a
    // request.
    rsx! {
        <style>(Raw::dangerously_create(CSS))</style>
        <style>(Raw::dangerously_create(super::shell::CSS))</style>
    }
}

#[cfg(test)]
mod tests {
    /// The design system's `button[type="submit"]` outranks its own
    /// variant classes, so a submit that asks to be secondary or ghost
    /// comes out primary black. Every form button on this console is a
    /// submit, which made almost the whole interface shout. These
    /// overrides are what put that back — losing them is silent.
    /// A `<pre>` is the one dark surface on the console, and the
    /// design system's single global `::selection` paints near-black
    /// text on it. An invitation link nobody can see while selecting
    /// it is one they cannot copy.
    #[test]
    fn a_dark_surface_has_its_own_selection_colour() {
        assert!(super::CSS.contains("pre ::selection"));
        assert!(super::CSS.contains("pre::selection"));
    }

    #[test]
    fn every_button_variant_beats_the_submit_default() {
        for variant in ["btn-secondary", "btn-ghost", "btn-danger"] {
            assert!(
                super::CSS.contains(&format!(r#"button[type="submit"].{variant}"#)),
                "{variant} would lose to button[type=\'submit\'] and render as primary"
            );
        }
    }
}
