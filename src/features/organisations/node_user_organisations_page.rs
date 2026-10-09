use hypertext::prelude::*;

use crate::domain::entities::tenant::Tenant;
use crate::features::organisations::texts::t;
use crate::web::views::layouts::layout;

/// One tile per organisation, and nothing to fill in on any of them.
///
/// **It was every form at once.** The domain, the quota, an invitation
/// with two selectors and the delete button were inline on each card, so
/// a node with four organisations was four of everything down one page
/// and the answer to "what is this organisation" was buried in the
/// controls for changing it. A tile now, and what it opens is the
/// organisation.
pub struct Organisations<'a> {
    pub cards: &'a [Card],
    pub error: Option<&'a str>,
}

pub struct Card {
    pub tenant: Tenant,
    pub projects: usize,
    pub people: usize,
}

pub fn render<'a>(page: &'a Organisations<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(t("Organisations"))</h1>
                <p class="tagline">(t("Who this node is shared out to."))</p>
            </div>
        </div>
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }

        @if page.cards.is_empty() {
            <section class="card stack">
                <p>(t("This node is not shared out to anybody yet."))</p>
                <p class="field-hint">(t("An organisation has its own projects, its own people and \
                     its own console. What stays this node's is the machine underneath \
                     it."))</p>
            </section>
        } @else {
            <div class="grid">
                @for card in page.cards {
                    <a class="card tile" href=(format!("/organisations/{}", card.tenant.id))>
                        <div class="split">
                            <p class="tile-name">(&card.tenant.name)</p>
                            @if card.tenant.domain.is_some() {
                                <span class="badge badge-success">
                                    <span class="dot dot-success"></span>(t("Console"))
                                </span>
                            } @else {
                                // Not a failure — an organisation without
                                // a domain works, it just has nowhere to
                                // be administered from yet. A red dot
                                // would be this page calling an unfinished
                                // setup a fault.
                                <span class="badge badge-info">
                                    <span class="dot dot-info"></span>(t("No domain"))
                                </span>
                            }
                        </div>
                        <p class="tile-detail">
                            (&card.tenant.slug)
                            (" · ")(card.projects)(t(" project(s)"))
                            (" · ")(card.people)(t(" person/people"))
                        </p>
                    </a>
                }
            </div>
        }

        <section class="card stack">
            <p class="card-label">(t("New organisation"))</p>
            <p class="field-hint">(t("Its projects and its people are its own: an \
                 administrator of one reaches nothing outside it, and this node's \
                 operator still sees everything the machine is running. Give it a \
                 domain to bring up its console."))</p>
            <form method="post" action="/organisations">
                <label for="name">(t("Name"))</label>
                <input id="name" name="name" autocomplete="off" required>
                <div class="actions">
                    <button type="submit">(t("Create"))</button>
                </div>
            </form>
        </section>
    }
}
