use hypertext::prelude::*;

use crate::domain::entities::account::Account;
use crate::domain::entities::tenant::Tenant;
use crate::domain::policies::presets;
use crate::features::organisations::texts::t;
use crate::web::views::layouts::layout;

/// The four things the node's operator decides about an organisation.
///
/// **And only these four.** Setting a role, adding somebody to a
/// project, naming a service — all of that used to be reachable from the
/// organisations page and none of it is the operator's: it is inside a
/// frontier they host rather than run. What is left is the machine's
/// side of the arrangement — the name it serves, who answers for it,
/// what it may take, and whether it stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganisationTab {
    Domain,
    Owner,
    Quota,
    Danger,
}

impl OrganisationTab {
    pub fn slug(self) -> &'static str {
        match self {
            OrganisationTab::Domain => "domain",
            OrganisationTab::Owner => "owner",
            OrganisationTab::Quota => "quota",
            OrganisationTab::Danger => "danger",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            OrganisationTab::Domain => t("Domain"),
            OrganisationTab::Owner => t("Owner"),
            OrganisationTab::Quota => t("Quota"),
            OrganisationTab::Danger => t("Danger"),
        }
    }

    pub fn all() -> [OrganisationTab; 4] {
        [
            OrganisationTab::Domain,
            // Second because it is the one thing that cannot be put off:
            // an organisation with no administrator cannot be given one
            // by anybody but the operator.
            OrganisationTab::Owner,
            OrganisationTab::Quota,
            OrganisationTab::Danger,
        ]
    }

    pub fn parse(slug: &str) -> Option<Self> {
        Self::all().into_iter().find(|tab| tab.slug() == slug)
    }

    pub fn path(self, tenant: &str) -> String {
        format!("/organisations/{tenant}/settings/{}", self.slug())
    }
}

pub enum Panel<'a> {
    Domain,
    Owner {
        owners: &'a [Account],
    },
    Quota,
    Danger {
        /// What deleting it would take with it. Said before the button
        /// rather than in the refusal after: a count somebody reads
        /// first is worth more than the same number afterwards.
        projects: usize,
        people: usize,
    },
}

pub struct OrganisationSettings<'a> {
    pub tenant: &'a Tenant,
    pub tab: OrganisationTab,
    pub panel: Panel<'a>,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a OrganisationSettings<'a>) -> impl Renderable + 'a {
    let tenant = page.tenant;
    let base = format!("/organisations/{}", tenant.id);
    rsx! {
        (layout::style_tag())
        <div class="stack-sm">
            <h1>(t("Settings"))</h1>
            <p class="slug-preview">(&tenant.name)</p>
        </div>

        <nav class="tabs" aria-label=(t("Settings"))>
            @for entry in OrganisationTab::all() {
                @if entry == page.tab {
                    <a href=(entry.path(&tenant.id)) class="tab active"
                       aria-current="page">(entry.label())</a>
                } @else {
                    <a href=(entry.path(&tenant.id)) class="tab">(entry.label())</a>
                }
            }
        </nav>

        @if let Some(message) = page.error {
            (layout::error_note(message))
        }

        @match &page.panel {
            Panel::Domain => {
                <section class="stack" id="domain">
                    <p class="card-label">(t("Domain"))</p>
                    <form method="post" action=(format!("{base}/domain")) class="card stack">
                        <label for="domain">(t("Domain"))</label>
                        <input id="domain" name="domain" type="text" autocomplete="off"
                               class="mono"
                               value=(tenant.domain.clone().unwrap_or_default())
                               placeholder="acme.example.com">
                        <p class="field-hint">(t("Where its console answers, and the suffix its \
                             service names hang off. It has to point at this node, and it \
                             cannot be one of this node's own names — the console an operator \
                             fixes things from must not be reachable only through somebody \
                             else's."))</p>
                        <p class="field-hint">(t("Empty takes the console away. Its projects and \
                             its people stay; there is nowhere to administer them from."))</p>
                        <div class="actions">
                            <button type="submit">(t("Save"))</button>
                        </div>
                    </form>
                </section>
            }
            Panel::Owner { owners } => {
                <section class="stack" id="owner">
                    <p class="card-label">(t("Answers for it"))</p>
                    <p class="field-hint">(t("The only people inside an organisation this node's \
                         operator decides about. Who else is in it, and what they reach, is \
                         theirs — from their own console."))</p>
                    <div class="card stack">
                        @if owners.is_empty() {
                            <p class="failure">(t("Nobody answers for this organisation. Until \
                                 somebody does, nothing inside it can be changed at all."))</p>
                        } @else {
                            <table>
                                <tbody>
                                    @for owner in *owners {
                                        <tr>
                                            <td class="mono">(&owner.username)</td>
                                            <td>
                                                <form method="post"
                                                      action=(format!("{base}/owners/{}/delete", owner.id))>
                                                    <button class="btn btn-ghost destructive btn-sm"
                                                            type="submit">(t("Remove"))</button>
                                                </form>
                                            </td>
                                        </tr>
                                    }
                                </tbody>
                            </table>
                        }
                        <form method="post" action=(format!("{base}/owners"))>
                            <p class="field-hint">(t("A link that works once and expires in seven \
                                 days. Whoever takes it chooses their own username and password, \
                                 and administers this organisation."))</p>
                            @if tenant.domain.is_none() {
                                // **The link lands on the organisation's
                                // own name**, because that is the door
                                // their account works at — the node's
                                // would sign them up and bounce them on
                                // the first click.
                                <p class="field-hint">(t("It has no domain yet, so the link will be \
                                     on this node's name — the account is made, and the console it \
                                     belongs to exists once you give the organisation one."))</p>
                            }
                            <div class="actions">
                                <button type="submit">(t("Invite an administrator"))</button>
                            </div>
                        </form>
                    </div>
                </section>
            }
            Panel::Quota => {
                <section class="stack" id="quota">
                    <p class="card-label">(t("Quota"))</p>
                    <form method="post" action=(format!("{base}/quota")) class="card stack">
                        <label for="memory">(t("Memory"))</label>
                        <input id="memory" name="memory" type="text" autocomplete="off"
                               class="mono"
                               placeholder=(t("no ceiling"))
                               value=(tenant.quota_memory.map(crate::infra::node::memory::human).unwrap_or_default())>

                        <label for="cpu">(t("CPU"))</label>
                        // **`cores_field`, not `cpu_label`.** One is what
                        // a number is called on a page and the other is
                        // what goes back into the box that produced it —
                        // and the difference bites: `2 cores` reads back
                        // as `2`, but `2000 cores` is what somebody typing
                        // millicores into an unlabelled box gets, and it
                        // reads back as two thousand of them.
                        <input id="cpu" name="cpu" type="text" autocomplete="off"
                               class="mono"
                               placeholder=(t("no ceiling"))
                               value=(tenant.quota_cpu.map(presets::cores_field).unwrap_or_default())>

                        <p class="field-hint">(t("Empty is no ceiling. Memory like 512 MB or 2 GB; \
                             CPU in cores like 0.5 or 2, or millicores with an m, like 500m."))</p>
                        <p class="field-hint">(t("It is summed over every copy of every service \
                             this organisation runs, wherever it runs — not what this machine \
                             holds. What a service may take is what counts against it: the \
                             ceiling is the reservation."))</p>
                        <div class="actions">
                            <button type="submit">(t("Save"))</button>
                        </div>
                    </form>
                </section>
            }
            Panel::Danger { projects, people } => {
                <section class="card stack">
                    <p class="card-label">(t("Danger zone"))</p>
                    <p class="tile-detail">(t("Deleting an organisation stops and removes every \
                         service it runs, on this node and on any other, and takes its projects \
                         and its people with it. There is no undo."))</p>
                    <p class="tile-detail">
                        (projects)(t(" project(s)"))(" · ")(people)(t(" person/people"))
                    </p>
                    <form method="post" action=(format!("{base}/delete")) class="stack">
                        <label for="confirm">(t("Type the name to confirm: "))(&tenant.slug)</label>
                        <input id="confirm" name="confirm" type="text" autocomplete="off"
                               class="mono" required placeholder=(&tenant.slug)>
                        <button class="btn btn-danger" type="submit">
                            (t("Delete organisation and everything in it"))
                        </button>
                    </form>
                </section>
            }
        }
    }
}
