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
/* Four corrections, and the reason they are all here is one thing the
 * dark palette does: it takes a token that reads well in light and
 * *inverts its role*, without revisiting what is painted on or beside
 * it. A light-mode chip becomes a dark-mode slab a shade off the card;
 * a red dark enough to read as text becomes a pink slab still carrying
 * white ink. Every one of these was invisible to whoever wrote it and
 * obvious to somebody using the console at night.
 *
 * The values are the same in both dark scopes — this one and the
 * `prefers-color-scheme` block below — so a change here needs the same
 * change there.
 */
[data-theme='dark'] {
  --c-fg:       224 218 209;  /* 12.64:1 on the canvas, down from 15.14 */
  --c-fg-faint: 158 152 143;  /* worst case 5.00:1, up from 2.60 */

  /* The ramp had no steps left in it. Correcting faint upwards brought
   * it to 158 152 143 and left muted at 160 154 145 — two tones two
   * units apart doing the work of two levels, so a table heading, a
   * hint and a slug were all the same grey and none of them read as
   * quieter than the next. Muted moves up to where it was meant to be:
   * 8.25:1 on the canvas, 6.89:1 on a card, and visibly above faint. */
  --c-fg-muted: 185 178 168;

  /* A secondary action you cannot find is not an action.
   *
   * `--c-action-soft` is 44 42 39 against a card's 34 33 31 — 1.12:1,
   * where the same token in light mode is a grey chip on white and
   * reads at once. "Save origin" was a label floating on the card with
   * no button under it. 88 84 78 puts the slab at 2.14:1 against the
   * card and keeps its label at 5.23:1, which is where this stops:
   * lighter finds the button and loses the word on it. */
  --c-action-soft: 88 84 78;

  /* An empty box you cannot see is not a control. The unchecked fill is
   * `--c-bg-contrast`, 44 42 39 — the same 1.12:1, so every unticked
   * box was a gap where a control should be. Reported on the edges
   * card, where the boxes are the whole of what the card does.
   *
   * 3.15:1, which is what WCAG asks of a control, and it can go the
   * whole way because nothing is written on it — unlike the button
   * above. */
  --c-control-empty: 112 108 101;
}

/* Light keeps what the design system chose; the token exists so the
 * rule below can be written once. */
:root {
  --c-control-empty: var(--c-bg-contrast);
}

/* `:not(:checked)` rather than a lower-specificity rule: the checked
 * fill is `--c-fg` and this must not be able to win against it. */
input[type="checkbox"]:not(:checked),
input[type="radio"]:not(:checked) {
  background: rgb(var(--c-control-empty));
}

/* A destructive action, in either theme.
 *
 * `.btn-danger` paints a fixed `--c-n-0` on a slab coloured
 * `--c-danger-fg` — and that token flips to a pale pink in dark so it
 * can be *read* as text, which left white on pink at 1.29:1. A delete
 * button nobody can read is the one button where that matters most.
 *
 * The ink is the inverse of its slab, exactly as the checkbox mark
 * below: unchanged in light, where `--c-fg-inverse` is that same white,
 * and near-black on the pink at 10.0:1 in dark.
 */
.btn-danger,
button[type="submit"].btn-danger {
  color: rgb(var(--c-fg-inverse));
}

/* A checked box, in either theme.
 *
 * The design system paints the mark a fixed `--c-n-0` on a box coloured
 * `--c-fg` — near-black in light, cream in dark. So in dark mode it was
 * white on cream: a checked box nobody could tell from an empty one,
 * which is a control that lies about the thing it controls. Reported on
 * the join terms screen, where every box is a decision.
 *
 * The mark is the inverse of its box, which is what it always meant. Here
 * rather than in `assets/wabot.css` for the reason the ramp above gives:
 * that file is vendored, and the real fix belongs upstream.
 */
input[type="checkbox"]::after,
input[type="radio"]::after {
  background: rgb(var(--c-fg-inverse));
}

/* Dark, followed from the operating system when nobody has chosen.
 *
 * The tokens are restated because the design system ships its dark
 * palette only under `[data-theme='dark']`, and CSS cannot make a
 * media query set an attribute. Keep in step with the vendored block
 * and with the corrections above; a `prefers-color-scheme` variant
 * belongs upstream.
 *
 * The semantic half was missing here, which is a whole class of "dark
 * mode looks wrong for some people and right for others": a running
 * badge, a failure, a warning and the brand ink all kept their light
 * values for anybody who had never touched the toggle, because that
 * leaves `data-theme` empty rather than `dark`. Nobody hits it who
 * chose their theme once.
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
    --c-fg-muted:    185 178 168;
    --c-fg-faint:    158 152 143;
    --c-fg-inverse:   17  16  15;
    --c-action:      244 242 238;
    --c-action-fg:    17  16  15;
    --c-action-soft:  88  84  78;
    --c-control-empty: 112 108 101;
    --c-brand-ink:   var(--c-brand-300);
    --c-brand-wash:   60  32  18;
    --c-success-bg:   30  56  38;
    --c-success-fg:  175 215 180;
    --c-warning-bg:   68  50  18;
    --c-warning-fg:  240 198 122;
    --c-danger-bg:    72  28  24;
    --c-danger-fg:   238 175 165;
    --c-info-bg:      28  46  68;
    --c-info-fg:     176 200 226;
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

/* The framework's two wrappers are unknown elements, so a browser lays
   them out **inline and unstyled**. That matters because a whole page
   body goes inside them: the outlet holds the view during boosted
   navigation, and a page with an island wraps everything in the host. So
   the page's sections stopped being children of `.shell` and the stack
   gap above never applied between them — the title row sat flush against
   the first card, on every page that had an island.

   `display: contents` rather than restating the flex column: these are
   hosts for behaviour, not layout boxes, and their children belong to
   whatever surrounds them. Restating it would be a second column that
   has to be kept in step with the first.

   Safe for the scripts: the elements stay in the DOM and
   `host.querySelector` still finds everything. Nothing here measures the
   host itself — `logs-live` measures the `<pre>` inside it. */
wabot-island,
wabot-outlet {
  display: contents;
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

/* The gap after a badge belongs to the badge, not to the sentence.
 *
 * It was a literal space inside the detail's text — and the stream writes
 * that text with `textContent`, so the space survived the first paint and
 * was gone two seconds later: an address sitting flush against the pill
 * beside it, on every row, but only after the page updated itself. The
 * same class of bug as the words themselves, one layer down.
 *
 * A margin cannot be overwritten by anything that replaces text. */
.badge + .tile-detail,
.badge + .mono,
.badge + .failure {
  margin-left: var(--sp-2);
}

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

/* A connection string, in one piece.
 *
 * It was a paragraph, so the browser broke it wherever it liked — and a
 * connection string with a line break in it is one nobody can use. This
 * scrolls sideways instead: the string stays whole, and the card keeps its
 * width whatever the domain's length.
 *
 * Four of them are rendered and one is shown, chosen by the radios above
 * with no script involved. That is the point: the console works with
 * scripting off, and the alternative to holding all four is a round trip
 * to move a radio button. `:has()` reads the state of a sibling input,
 * which is the whole mechanism.
 */
.dsn-value {
  margin: 0;
  padding: var(--sp-3) var(--sp-4);
  background: rgb(var(--c-bg-sunken));
  border-radius: var(--r-md);
  font-family: var(--font-mono);
  font-size: 0.82rem;
  line-height: 1.5;
  color: rgb(var(--c-fg-body));
  white-space: pre;
  overflow-x: auto;
}
.dsn:has(#dsn-full:checked):has(#dsn-primary:checked) [data-dsn="primary-full"],
.dsn:has(#dsn-full:checked):has(#dsn-pool:checked) [data-dsn="pool-full"],
.dsn:has(#dsn-short:checked):has(#dsn-primary:checked) [data-dsn="primary-short"],
.dsn:has(#dsn-short:checked):has(#dsn-pool:checked) [data-dsn="pool-short"] {
  display: flex;
}

/* And when the name is not a choice there is no group to match on: one
   spelling exists, so the copy radio alone decides. Written as "no
   `#dsn-full` in this block" rather than a class the server sets, because
   the markup already says it — the radio is there or it is not. */
.dsn:not(:has(#dsn-full)):has(#dsn-primary:checked) [data-dsn^="primary-"],
.dsn:not(:has(#dsn-full)):has(#dsn-pool:checked) [data-dsn^="pool-"],
.dsn:not(:has(#dsn-short)):has(#dsn-primary:checked) [data-dsn^="primary-"],
.dsn:not(:has(#dsn-short)):has(#dsn-pool:checked) [data-dsn^="pool-"] {
  display: flex;
}
/* A group's label belongs to the block under it — the rule the form
   labels above already follow, and here the design system works against
   it. `.card-label` carries a `margin-bottom` of its own *on top of* the
   stack's gap, so there was 20px below the words and 8px above them: each
   label floated between two groups instead of heading one.

   That reads fine on a card with a single heading, which is every other
   card in this console — it is only where labels alternate with content,
   as they do here, that the asymmetry shows. So the fix is scoped rather
   than applied to `.card-label` everywhere. */
.dsn .card-label { margin-bottom: 0; }
.dsn .dsn-values + .card-label,
.dsn .field-hint + .card-label {
  margin-top: var(--sp-3);
}

.dsn-pick { flex-wrap: wrap; gap: var(--sp-5); align-items: center; }
.dsn-group { display: flex; flex-wrap: wrap; gap: var(--sp-4); align-items: center; }

/* The string and its button on one line.
 *
 * The button was positioned over the block and sat on top of the string —
 * which for a string that scrolls is worse than no button, because it
 * covers exactly the end somebody is trying to read. Beside it instead, and
 * the string takes the room that is left.
 */
/* A line is shown unless something hides it. A database's four are
   hidden by `[data-dsn]` and revealed one at a time by the radios; a
   service's are a list and every one of them is on screen, which is the
   difference between "which do you want" and "what does this offer". */
.dsn-line { display: flex; align-items: center; gap: var(--sp-3); }
.dsn-line[data-dsn] { display: none; }
.dsn-line .dsn-value { flex: 1 1 auto; min-width: 0; }
.dsn-line .btn { flex: 0 0 auto; }

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

/* What a container is saying.
   Its own scroller rather than the page's, so the panel keeps its place
   on the screen while the text moves inside it — a log that grew the
   page would push everything else off and put the newest line wherever
   the window happened to end.
   `pre-wrap`, not `pre`: a stack trace or a long SQL statement wraps
   rather than making the whole panel scroll sideways, and a horizontal
   scrollbar under a live feed is a thing nobody ever reaches for. */
.log {
  margin: 0;
  max-height: 60vh;
  overflow-y: auto;
  overflow-x: hidden;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  line-height: 1.5;
  color: rgb(var(--c-fg));
  background: rgb(var(--c-bg-sunken));
  padding: var(--sp-3);
  border-radius: var(--r-sm);
  /* Reading position is the bottom, and it should stay there while the
     text grows. The script only scrolls when it was already there. */
  scrollbar-gutter: stable;
}

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
/* The box and the dot keep their size inside a `.check` label.
 *
 * The width is restated because the design system sets `width: 100%` on
 * every input and then 1.05rem on these two, and in a flex label the
 * first one wins — the checkbox rule has been here for that reason since
 * before radios were used in one. A radio without it renders as a
 * rounded rectangle: the height is right, the width is not, and 50% of a
 * rectangle is a pill. */
.check input[type="checkbox"],
.check input[type="radio"] {
  width: 1.05rem;
  height: 1.05rem;
  /* And no padding, which is the whole of why a radio was a pill.
   *
   * The design system pads every input — `0.625rem 0.75rem` — and then
   * sizes these two without resetting it. With `box-sizing: border-box`
   * the padding is a floor, so the box is at least 1.5rem wide by 1.25rem
   * tall: wider than tall, and 50% of that is an ellipse. Setting the
   * width alone, which is what was here, could never have worked. */
  padding: 0;
  flex: 0 0 auto;
}
.check input[type="radio"] { border-radius: 50%; }

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
    /// The framework's wrappers are unknown elements, so a browser lays
    /// them out inline and unstyled — and a page with an island puts its
    /// whole body inside one. Without this the page's sections stop
    /// being children of `.shell`, the stack gap never applies between
    /// them, and the title row sits flush against the first card. It was
    /// like that on every island page until somebody looked at one.
    ///
    /// Losing this rule is silent in exactly the same way, which is why
    /// it is asserted rather than trusted.
    #[test]
    fn the_frameworks_wrappers_do_not_break_the_page_stack() {
        let rule = super::CSS
            .split("wabot-island")
            .nth(1)
            .expect("the wrappers are styled");
        assert!(
            rule.contains("wabot-outlet"),
            "only one of the two: {rule:.80}"
        );
        assert!(
            rule.split('}')
                .next()
                .unwrap_or_default()
                .contains("display: contents"),
            "they generate a box again: {rule:.120}"
        );
    }

    #[test]
    fn a_dark_surface_has_its_own_selection_colour() {
        assert!(super::CSS.contains("pre ::selection"));
        assert!(super::CSS.contains("pre::selection"));
    }

    /// Dark mode is written twice — once for the toggle and once for
    /// the operating system's preference — because a media query cannot
    /// set an attribute. So a correction made in one scope is invisible
    /// to half the people using the console, and which half depends on
    /// whether they ever touched the toggle. Four tokens were in that
    /// state at once: the semantic set, which meant a running badge kept
    /// its light-mode colours on a dark page.
    #[test]
    fn the_two_dark_palettes_agree() {
        let chosen = tokens(super::CSS, "[data-theme='dark'] {");
        let followed = tokens(super::CSS, ".app-shell:not([data-theme='light']) {");
        assert!(!chosen.is_empty(), "the explicit dark block was not found");

        for (name, value) in &chosen {
            let there = followed.iter().find(|(other, _)| other == name);
            assert_eq!(
                there.map(|(_, value)| value.as_str()),
                Some(value.as_str()),
                "{name} differs between the two dark scopes"
            );
        }
    }

    /// The custom properties a block declares, comments stripped and
    /// whitespace flattened, so two spellings of the same colour
    /// compare equal.
    #[cfg(test)]
    fn tokens(css: &str, selector: &str) -> Vec<(String, String)> {
        let body = css
            .split(selector)
            .nth(1)
            .expect("the block is there")
            .split('}')
            .next()
            .expect("the block closes");

        let mut plain = String::new();
        let mut rest = body;
        while let Some(start) = rest.find("/*") {
            plain.push_str(&rest[..start]);
            let end = rest[start..].find("*/").expect("the comment closes");
            rest = &rest[start + end + 2..];
        }
        plain.push_str(rest);

        plain
            .split(';')
            .filter_map(|declaration| declaration.split_once(':'))
            .map(|(name, value)| {
                (
                    name.trim().to_string(),
                    value.split_whitespace().collect::<Vec<_>>().join(" "),
                )
            })
            .filter(|(name, _)| name.starts_with("--"))
            .collect()
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
