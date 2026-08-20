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
/// A moment, as a moment.
///
/// **The exception the comment below names.** Everywhere else on this
/// console a time answers "did that just happen", and relative is the
/// better answer. A recovery window is the one place it is not: somebody
/// reading it is about to *type a time* — "restore to 14:32" — and
/// "between 3 days ago and 2 minutes ago" cannot be typed into anything.
///
/// UTC, and labelled. The node keeps UTC, the operator may not, and a
/// timestamp with no zone on the page somebody uses to choose a recovery
/// point is an hour of somebody's data decided by a guess.
pub fn exactly(at_ms: i64) -> String {
    let Ok(moment) = time::OffsetDateTime::from_unix_timestamp(at_ms / 1000) else {
        return "an unreadable time".into();
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        moment.year(),
        u8::from(moment.month()),
        moment.day(),
        moment.hour(),
        moment.minute(),
    )
}

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

/* The same rule for the line under a replica's badge, which is grey while
   the copy is fine and red when it is not — so it cannot be `.failure`
   from the start. It is rendered empty and always present for the reason
   above, and `console.js` swaps its class along with its text. The
   principle was already written down here and `placement_state` had not
   followed it: a copy that failed after the page loaded flipped its badge
   live and lost the reason until a refresh. */
.detail-line:empty { display: none; }

/* And the space above it. The shared rule further up gives `.badge +
   .failure` a *left* margin, which is right for a detail sitting inline
   beside a badge and does nothing for one below it — this is a block, so
   a left margin was an indent and the red band sat flush against the
   badge. Reported by Jorge.
 *
 * The line is always below the badge now, whatever the state, which is
 * also what lets it keep its element across a change of severity: a
 * detail that moved from beside the badge to under it depending on
 * whether things were going well would move the row's contents around
 * under somebody reading them. */
.badge + .detail-line {
  margin-left: 0;
  margin-top: var(--sp-1);
}

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

/* Same asymmetry, one element down. `.field-hint` carries a *negative*
   top margin and a large bottom one, because under a field that is
   exactly right — it belongs to the input above it. Inside a flex stack
   that already spaces its children, the negative one pulls the line up
   against whatever it follows and the bottom one adds to the gap: so a
   hint opening a disclosure sat on the summary, and every hint after a
   command block was twice as far from it as the block was from the line
   above. The stack's gap is the spacing; the margins are not. */
.dsn .field-hint { margin: 0; }

/* A block somebody opens once and then scrolls past — the connection
   strings, and how to push. The design system styles neither, so the
   marker had no pointer and the first line inside started flush against
   the word that opens it. */
details > summary {
  cursor: pointer;
  font-weight: 500;
}
details[open] > summary { margin-bottom: var(--sp-4); }

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
/* The button is exactly as tall as the block it stands beside, and square,
   because it is only its icon.
 *
 * `--dsn-height` is one line of `.dsn-value`: its font-size times its
 * line-height, plus its padding twice. It is arithmetic over the same
 * numbers the block itself uses, so it is declared here — one place, read
 * by the button — rather than a measured pixel value that stops being
 * true the next time the block's padding changes.
 *
 * `flex-start` rather than `center` for the block that holds more than one
 * line: the push commands are two or three, and a button centred against
 * them floats in the middle of a paragraph. Aligned to the top it sits on
 * the first line, which is where a control belongs. For a single line the
 * two are identical, since the heights are the same by construction. */
.dsn-line {
  --dsn-height: calc(0.82rem * 1.5 + var(--sp-3) * 2);
  display: flex;
  align-items: flex-start;
  gap: var(--sp-3);
}
.dsn-line[data-dsn] { display: none; }
.dsn-line .dsn-value { flex: 1 1 auto; min-width: 0; }
.dsn-line .btn { flex: 0 0 auto; }
/* Overriding `.btn-icon`'s 2.25rem, which is sized for the header's
   controls and has nothing beside it to line up with. The glyph keeps the
   one size every icon button uses — the box is taller here, and 22px in it
   is the same proportion the shell's buttons already read at. */
.dsn-copy { width: var(--dsn-height); height: var(--dsn-height); }
/* One limit worth knowing: `.dsn-value` scrolls sideways, and on a
   platform whose scrollbars take space rather than overlay the content, a
   string long enough to scroll makes the block taller than this by the
   scrollbar's height. `align-items: stretch` would follow it exactly and
   was the first shape of this — it cannot be used, because the same rule
   governs the push commands, which are three lines, and a button stretched
   down three lines is not a control anybody aims at. */

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
    /// No page offers a button back to where it came from.
    ///
    /// **The breadcrumb above the title is the way back**, and one thing
    /// must not be doable two ways — a second control for it reads as a
    /// second destination. This was reported three times: a "Back to
    /// project" button on a service, a "Back to service" on its settings,
    /// and before those the duplicated create buttons on an empty project.
    /// Three reports of one principle is where a rule earns a test.
    ///
    /// It reads the console's own sources, so a fourth one fails here
    /// rather than in somebody's screenshot. `Cancel` on a form is a
    /// different thing and stays: it abandons an edit, which the
    /// breadcrumb does not say anything about.
    #[test]
    fn no_page_offers_a_way_back_the_breadcrumb_already_is() {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src/console"))
            .expect("the console's own directory")
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".rs") {
                continue;
            }
            let source = std::fs::read_to_string(entry.path()).expect("read");
            for (number, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                // Only what a page renders. A comment saying where a
                // handler redirects to is prose about the same subject
                // and not a control.
                if trimmed.starts_with("//") {
                    continue;
                }
                // Rendered *content*, which in `rsx!` is `>(…)`. The
                // first version of this matched the text anywhere and
                // caught the breadcrumb arrow's own `title` and
                // `aria-label` — which say "Back to <where>" and should,
                // because that is the accessible name of the one control
                // this rule exists to protect. The label somebody reads is
                // what must not be duplicated; the name a screen reader
                // announces for the arrow is the arrow.
                if line.contains(">(t(\"Back to") {
                    found.push(format!("{name}:{}", number + 1));
                }
            }
        }
        assert!(
            found.is_empty(),
            "these render a way back the breadcrumb already is: {found:?}"
        );
    }

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
    ///
    /// Both blocks are in `assets/wabot.css` now, which is where a token
    /// is defined. This reads the stylesheet the pages actually load —
    /// not a copy of it — so the file being editable does not make it
    /// unguarded.
    /// A button that is only an icon still has a name to read out.
    ///
    /// The copy control was the translated word `Copy`; it is a glyph now,
    /// on Jorge's ask, and a glyph is nothing to a screen reader and
    /// nothing to hover. So the word moved to `aria-label` rather than
    /// being dropped — the one cost of the icon, and the one that is easy
    /// to lose in a later simplification of the island.
    ///
    /// Read out of the asset because that is what the node serves. There
    /// is no JavaScript in the test suite, so this is the only place the
    /// claim can be made at all.
    #[test]
    fn the_copy_button_has_a_name_and_not_only_a_glyph() {
        let script = include_str!("../../assets/console.js");
        let island = script
            .split("wabot.island('copy'")
            .nth(1)
            .expect("the copy island");
        // Up to the next island, so a name set somewhere else cannot pass
        // for this one.
        let island = island.split("wabot.island(").next().expect("its body");

        // The *set*, not any mention: a first version of this asked for
        // `aria-label` anywhere in the island and passed with the
        // assignment deleted, because reading the attribute back mentions
        // it too. A test that survives the deletion it exists to catch is
        // worse than none.
        assert!(
            island.contains(concat!("setAttribute('", "aria-label'")),
            "an icon with nothing to read out: {island}"
        );
        // An icon rather than a word — the glyph is a constant above, so
        // the island is asked about the glyph and the file about the SVG.
        assert!(
            island.contains("COPY_GLYPH"),
            "the button renders no glyph: {island}"
        );
        let glyph = script
            .split("COPY_GLYPH =")
            .nth(1)
            .expect("the glyph")
            .split(';')
            .next()
            .expect("its value");
        assert!(glyph.contains("<svg"), "and the glyph is not one: {glyph}");
        assert!(
            glyph.contains("currentColor"),
            "an icon that does not follow the theme: {glyph}"
        );
        // The translated strings still reach it. The markup carries them,
        // and dropping them would leave the English fallback on a Spanish
        // page — which `every_string_the_console_asks_for_is_translated`
        // cannot see, because the words are attributes rather than text.
        assert!(
            island.contains("copyLabel") && island.contains("copiedLabel"),
            "the translated words are gone: {island}"
        );
    }

    #[test]
    fn the_two_dark_palettes_agree() {
        let sheet = include_str!("../../assets/wabot.css");
        let chosen = tokens(sheet, "[data-theme='dark'] {");
        let followed = tokens(sheet, ".app-shell:not([data-theme='light']) {");
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

    /// A submit with a variant class must not be styled as primary.
    ///
    /// `button[type='submit']` is 0,1,1 and `.btn-secondary` is 0,1,0,
    /// so the bare selector wins and every submit came out black
    /// whatever it asked for — on a console where almost every action
    /// is a form, almost every button was shouting.
    ///
    /// It was patched by restating each variant at a higher
    /// specificity, which had to be repeated for each new one and was
    /// checked here by name. The cause is fixed instead: `:not([class])`
    /// on the primary selector, which is the idiom the design system's
    /// own secondary rule already used. So what this asserts is the
    /// guard, not the patches — a variant added tomorrow is covered
    /// without touching this test.
    #[test]
    fn a_classed_submit_is_not_the_primary_button() {
        let sheet = include_str!("../../assets/wabot.css");
        for rule in sheet.split('}') {
            let Some((selectors, body)) = rule.split_once('{') else {
                continue;
            };
            // Only the rules that *paint*. `:disabled` reaches every
            // submit on purpose — it sets opacity and a cursor, which
            // a disabled button of any variant should have, and there
            // is nothing there for a variant to want back.
            if !body.contains("background") && !body.contains("color") {
                continue;
            }
            for selector in selectors.split(',') {
                let selector = selector.trim();
                assert!(
                    !selector.starts_with("button[type='submit']")
                        || selector.contains(":not([class])"),
                    "{selector:?} paints every submit, variants included"
                );
            }
        }
    }
}
