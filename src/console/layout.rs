//! The chrome every console page shares, and the CSS it needs.
//!
//! One place, because a header that drifts between pages is how a
//! console starts feeling like three consoles.

use hypertext::prelude::*;
use hypertext::Raw;

use super::assets;
use super::language::t;

/// Declare the head every console page needs.
///
/// Called by each view rather than by a layout function, because the
/// framework assembles the document and a view's only way in is the
/// render scope.
pub fn head(title_text: &str) {
    use wabot::ui::hypertext::{link, script, style, title};

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
    // Deferred by being a module, so it runs after the document is
    // parsed and finds the forms it is about. Nothing waits on it: the
    // page is already complete and correct when it arrives.
    script(format!("{}/console.js", assets::MOUNT));
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
        <p class="form-error"><strong>(t("Error: "))</strong>(message)</p>
    }
}

/// Page-specific layout. Colour, type and radii come from the design
/// system's tokens; this is arrangement, which is the console's own
/// business. No borders, no shadows, no hover states — separation is
/// background contrast.
pub const CSS: &str = r#"
/* The quiet text tones, corrected in both themes.
 *
 * The design system ships `--c-fg-faint` at a value that fails WCAG AA
 * against every surface this console paints it on — 2.0:1 to 2.5:1 in
 * light, 2.6:1 to 3.3:1 in dark, where the floor for body text is
 * 4.5:1. That is not decorative: faint is what renders a project's
 * slug, an empty-state line and every placeholder. `--c-fg-muted`
 * failed the same way in light only.
 *
 * Overridden here rather than in `assets/wabot.css` because that file
 * is vendored — editing it forks the copy and the next sync silently
 * takes the fix away. The ramp belongs to the whole design system, so
 * the real fix is upstream; this holds until then.
 */
:root {
  --c-fg-muted: 88 83 76;   /* worst case 6.06:1 */
  --c-fg-faint: 104 99 92;  /* worst case 4.73:1 */
}

/* Dark, chosen explicitly. Two corrections, and only two — the rest of
 * the dark palette is the design system's and it measures well.
 *
 * `--c-fg` comes down from 15.1:1 to 12.6:1 against the canvas. There
 * is no upper bound in WCAG, but near-maximum contrast on a dark
 * background is what makes light text look like it is vibrating, and
 * the people who feel it worst are the ones reading a deploy console
 * at two in the morning. 12.6:1 is still far above AAA.
 *
 * The canvas itself is left alone at #1A1918. It is a deliberate warm
 * charcoal rather than black, which is the part of "easy on the eyes"
 * the design system already got right.
 */
[data-theme='dark'] {
  --c-fg:       224 218 209;  /* 12.64:1 on the canvas, down from 15.14 */
  --c-fg-faint: 158 152 143;  /* worst case 5.00:1, up from 2.60 */
}

/* Dark, followed from the operating system when nobody has chosen.
 *
 * The tokens are restated because the design system ships its dark
 * palette only under `[data-theme='dark']`, and CSS cannot make a
 * media query set an attribute. Keep in step with the vendored block;
 * a `prefers-color-scheme` variant belongs upstream.
 */
@media (prefers-color-scheme: dark) {
  .app-shell:not([data-theme='light']) {
    --c-bg:          26 25 24;
    --c-bg-raised:   34 33 31;
    --c-bg-sunken:   21 20 19;
    --c-bg-contrast: 44 42 39;
    --c-bg-inverse:  244 242 238;
    --c-fg:          224 218 209;
    --c-fg-body:     214 210 202;
    --c-fg-muted:    160 154 145;
    --c-fg-faint:    158 152 143;
    --c-fg-inverse:   17  16  15;
    --c-action:      244 242 238;
    --c-action-fg:    17  16  15;
    --c-action-soft:  44  42  39;
    --c-brand-wash:   60  32  18;
  }
}

/* The theme attribute sits on the shell rather than the document root:
 * the framework assembles the document, and a view has no way to reach
 * that element. So the shell paints the canvas itself, or the strip a
 * rubber-band scroll reveals would stay the other theme's colour.
 */
.app-shell {
  background: rgb(var(--c-bg));
  color: rgb(var(--c-fg-body));
  color-scheme: light dark;
}

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

/* A control that is only its icon. Square, so it reads as one thing to
   press rather than a word that lost its label.

   Its glyph is larger than the gear's, and deliberately: the gear sits
   beside its word and takes the word's size, while this one carries the
   whole control on its own. Matching them made this look shrunken. */
.btn-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 2.25rem;
  height: 2.25rem;
  padding: 0;
}
/* And the glyph is sized here rather than by the `width` on each SVG.
   The box was widened once without the glyph following it, so an
   18-pixel icon sat in a 36-pixel button and read as shrunken — twice.
   One rule that every icon button obeys is the fix; the attributes on
   the SVGs are what a browser uses before this stylesheet arrives. */
.btn-icon svg {
  width: 1.375rem;
  height: 1.375rem;
}

/* Which half of the theme toggle is real, while nobody has chosen.
   Light or dark is the account's answer once it presses; until then the
   machine's answer is the only one there is, and the *server* cannot
   read it. CSS can — so both buttons are rendered and this picks, with
   no JavaScript involved. See `Frame::toggle`. */
.when-dark { display: none; }
@media (prefers-color-scheme: dark) {
  .when-light { display: none; }
  .when-dark { display: inline-flex; }
}

/* The gear beside its word in the top nav. The word stays: an icon
   alone in a two-item nav is a guess, and there is room for both. */
.topbar nav a { display: inline-flex; align-items: center; gap: var(--sp-2); }

/* Who is in a project, and what they may do. Two fields and a button,
   each labelled — the row of three bare controls it replaces read as a
   search box somebody had mislabelled. */
.add-person {
  display: flex;
  gap: var(--sp-3);
  align-items: flex-end;
  flex-wrap: wrap;
}
.add-person > div { display: flex; flex-direction: column; gap: var(--sp-1); }
.add-person > div:first-child { flex: 1 1 14rem; }

/* The cell a destructive button sits in, so it does not stretch. */
.row-actions { width: 1%; white-space: nowrap; }

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

/* The state column keeps its width whatever the word is.
 *
 * "Not deployed" wrapped to two lines where "Running" did not, so the
 * row grew and the column moved — and with the state updating in place
 * that becomes a table that jumps while somebody is reading it. Wide
 * enough for the longest answer, and the badge never wraps.
 */
/* Fixed widths so a column does not resize when its content does. The
   state and the address both change in place now, and a table that
   reflows on every update is one nobody can read while it works. */
.state { width: 9rem; }
.address { width: 9rem; }

/* An icon button is square, and its glyph is centred rather than
   sitting on the text baseline it no longer has. */
.btn.icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0.35rem 0.6rem;
}
.state .badge { white-space: nowrap; }

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
/* The node page keeps an empty one in the markup for the stream to
   write into — the script only replaces text that is already there, so
   an element that appears on failure could never appear at all. Empty
   means nothing failed, and a bare red band saying nothing is worse
   than no band. */
.failure:empty { display: none; }

/* What `console.js` hides. `!important` because these sit inside flex
   and grid containers, whose `display` on the child would otherwise
   win over the `hidden` attribute's user-agent rule — the field would
   stay on screen with nothing but the attribute to say it should not
   be. Nothing else in this stylesheet uses `!important`. */
[hidden] { display: none !important; }
/* The same job for markup the server renders. The `hidden` attribute
   cannot be used there: it is boolean by presence, so writing it with a
   falsey value hides the element just as thoroughly. */
.is-hidden { display: none !important; }

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

/* Where this page sits, and the way out. Above the title for the same
   reason `.crumb` is: it is orientation, not content, so it must not
   compete with the heading somebody came to read.

   The frame draws it on every page from the path — see `shell::crumbs`
   — so this is the only place its shape is decided. */
/* Centred, not baseline-aligned. The back control holds an SVG and no
   text, so it has no baseline of its own — a browser falls back to its
   bottom edge, and the arrow floated above the trail it belongs to. The
   bug arrived the moment the control stopped being a word. */
.crumbs {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  margin: 0 0 var(--sp-4);
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-muted));
}
.crumbs ol {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--sp-2);
  margin: 0;
  padding: 0;
  list-style: none;
  min-width: 0;
}
/* The separator is generated rather than written into the markup: a
   `/` between two links is punctuation, and punctuation that is part of
   the document gets read out and selected with the text. */
.crumbs li + li::before {
  content: "/";
  margin-right: var(--sp-2);
  color: rgb(var(--c-fg-faint));
}
.crumbs a { color: rgb(var(--c-fg-muted)); }
/* Where you are, in the trail. Not a link, and not louder than the
   heading right underneath it either. */
.crumbs [aria-current="page"] { color: rgb(var(--c-fg)); }

/* The one control in the bar, and an icon rather than a word: its
   destination is always the crumb it sits next to, so spelling it out
   printed the same name twice on every page one level deep.

   It carries `.btn-icon` for the hit area — a bare arrow beside a text
   link is a few pixels wide next to something far easier to hit — and
   is pulled left so the square sits flush with the page's edge instead
   of indenting the trail. */
.crumb-back {
  flex: none;
  margin-left: calc(var(--sp-3) * -1);
}

/* A long trail on a narrow screen wraps rather than pushing the page
   sideways; the back link stays on the first line. */
@media (max-width: 40rem) {
  .crumbs { flex-wrap: wrap; gap: var(--sp-2); }
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

/* A switch and what it means, side by side. The explanation is not a
   hint under a field — it is the difference between "cannot" and "will
   not", which is the whole of what the switch decides. */
.capability {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-3);
}

.capability > span {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

/* How many copies, and where new ones go. Side by side because they
   are one decision read left to right — a count with a destination —
   and stacked on a narrow screen rather than squeezed. */
.placement-count {
  display: flex;
  gap: var(--sp-4);
  flex-wrap: wrap;
  align-items: flex-end;
}

.placement-count > div {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

/* Choosing who answers for a name. On its own line under the address
   it belongs to — it is about that hostname and nothing else in the
   row, and reading it beside the address is what makes that obvious. */
.served-by {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  flex-wrap: wrap;
  flex-basis: 100%;
  font-family: var(--font-sans);
}

.served-by-node {
  display: flex;
  align-items: center;
  gap: var(--sp-1);
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
