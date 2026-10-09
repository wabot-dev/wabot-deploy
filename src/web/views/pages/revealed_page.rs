use hypertext::prelude::*;

use crate::web::views::layouts::layout;
use crate::web::views::texts::language::t;
use crate::web::views::texts::sentences;

/// A secret, shown once, on a page of its own.
///
/// **It used to be a card bolted to the top of whatever page minted it.**
/// The invite page came back with the token above its own form, the
/// organisation came back with the link above its quota — so the one
/// thing somebody has to act on *now* arrived surrounded by the controls
/// they had just finished using, and reloading to get rid of it is what
/// spends the nonce. Reported by Jorge: "es muy extraña la experiencia".
///
/// One page, one thing on it, and the breadcrumb's arrow goes back to
/// where it came from. The page draws no button for that — see
/// `no_page_offers_a_way_back_the_breadcrumb_already_is`.
pub struct Revealed<'a> {
    pub what: What<'a>,
    pub secret: &'a str,
}

/// Which secret this is, and therefore what the page says about it.
///
/// A kind rather than the prose itself, because the words are chosen
/// inside the render scope: the language is set around `rsx!` and a
/// controller that stashed a sentence would have stashed it in whichever
/// language nobody asked for.
pub enum What<'a> {
    /// The token itself, pasted into the other node.
    JoinToken,
    /// A link into an organisation, on that organisation's own name.
    InvitationInto(&'a str),
    /// A link into this console.
    Invitation,
    /// A `docker login` for one project's registry.
    PushToken,
}

impl What<'_> {
    fn heading(&self) -> &'static str {
        match self {
            What::JoinToken => t("Paste this into the other node"),
            What::InvitationInto(_) | What::Invitation => t("Send this link"),
            What::PushToken => t("Log in with this"),
        }
    }
}

pub fn render<'a>(page: &'a Revealed<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <div class="stack-sm">
            <h1>(page.what.heading())</h1>
        </div>

        <section class="card stack">
            // **The one moment it exists in clear.** Said above the
            // secret rather than under it: somebody who reads it after
            // copying has already decided whether to keep it.
            <p class="field-hint">(t("This is the only time it is shown. What is stored is a \
                 hash of it, so no page can show it again — copy it now, and make \
                 another if you lose it."))</p>
            (copyable(page.secret))
            @match &page.what {
                What::JoinToken => {
                    // **The token alone, not the command.** It is pasted
                    // into the other node's console far more often than
                    // into its terminal, and a prefix somebody has to
                    // delete first is worse than one they can type. The
                    // command is named here for the terminal case, and
                    // not translated — a terminal does not speak
                    // Spanish.
                    <p class="field-hint">(t("On the other node's console: Network, then Join a \
                         network. Or on its terminal:"))</p>
                    <p class="field-hint mono">("wabot-deploy join <token>")</p>
                    <p class="field-hint">(t("It works once and expires in 24 hours. That machine \
                         has to be a node already: install it there first, then join."))</p>
                }
                What::InvitationInto(name) => {
                    <p class="field-hint">(sentences::invitation_into(name))</p>
                }
                What::Invitation => {
                    <p class="field-hint">(t("It works once and expires in seven days. They \
                         choose their own username and password."))</p>
                }
                What::PushToken => {
                    <p class="field-hint">(t("Use it as the password wherever this project's \
                         images are pushed from. It is nobody's password: revoking it changes \
                         nothing else."))</p>
                }
            }
        </section>
    }
}

/// The block, with the copy button the island puts on it.
///
/// One island around one value: the runtime re-hydrates it after every
/// boosted swap, which an inline `<script>` on this page would not
/// survive — and this page is always arrived at by one.
///
/// **`.dsn-line` is not decoration.** The island appends the button as a
/// sibling of the block, and that rule is what makes the pair a row:
/// without it the button laid itself out underneath as a full-width bar,
/// which is what shipped. Reported by Jorge from the page.
fn copyable(value: &str) -> impl Renderable + '_ {
    let (copy, copied) = (t("Copy"), t("Copied"));
    let inner = rsx! {
        <div class="dsn-line">
            <pre class="dsn-value" data-copy
                 data-copy-label=(copy) data-copied-label=(copied)>(value)</pre>
        </div>
    }
    .render()
    .into_inner();

    // XSS SAFETY: the markup is this function's own, and `value` is
    // rendered escaped by `rsx!` above before it is wrapped.
    wabot::ui::hypertext::island(
        "copy",
        &serde_json::json!({}),
        hypertext::Raw::dangerously_create(&inner),
    )
}
