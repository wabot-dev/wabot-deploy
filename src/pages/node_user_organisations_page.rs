use hypertext::prelude::*;

use crate::console::language::t;
use crate::console::language::word;
use crate::console::layout;
use crate::platform::presets;

use crate::entities::project::Project;
use crate::platform::tenants::Tenant;

pub struct Organisations<'a> {
    pub cards: &'a [(Tenant, Vec<Project>)],
    pub query_error: Option<&'a str>,
    pub query_invited: Option<&'a str>,
}

pub fn render<'a>(page: &'a Organisations<'a>) -> impl Renderable + 'a {
    let cards = page.cards;
    rsx! {
        // The console's own CSS, which every view emits into its
        // own body. `wabot.css` is the design system's tokens;
        // the shell, the cards and this page's layout are in
        // here, and a page without it renders complete and
        // unstyled — which is exactly how it looked.
        (layout::style_tag())
        <div class="stack">
            <div class="split">
                <div class="stack-sm">
                    <h1>(t("Organisations"))</h1>
                    <p class="slug-preview">(t("Who this node is shared out to"))</p>
                </div>
            </div>
            @if let Some(message) = &page.query_error {
                (layout::error_note(message))
            }
            // The one moment the token exists in clear, shown by
            // the page that made it and never again — the same
            // block the people page has, because an invitation
            // minted here has to be readable here.
            @if let Some(link) = &page.query_invited {
                <section class="card stack">
                    <p class="card-label">(t("Send this link"))</p>
                    <p class="field-hint">(t("It works once and expires in seven days. The node will not \
                         show it again — it is not stored in a form anybody can read \
                         back."))</p>
                    <pre><code>(link)</code></pre>
                </section>
            }

            <section class="card stack">
                <p class="card-label">(t("New organisation"))</p>
                <p class="field-hint">(t("Its projects and its people are its own: an \
                     administrator of one reaches nothing outside it, and this \
                     node's operator still sees everything the machine is \
                     running. Give it a domain to bring up its console."))</p>
                <form method="post" action="/organisations">
                    <label for="name">(t("Name"))</label>
                    <input id="name" name="name" autocomplete="off" required>
                    <div class="actions">
                        <button type="submit">(t("Create"))</button>
                    </div>
                </form>
            </section>

            @for (tenant, held) in cards {
                <section class="card stack">
                    <p class="card-label">(&tenant.name)</p>
                    <p class="slug-preview">(&tenant.slug)(" · ")(held.len().to_string())(t(" project(s)"))</p>

                    <form method="post" class="stack"
                          action=(format!("/organisations/{}/domain", tenant.id))>
                        <label for=(format!("domain-{}", tenant.id))>(t("Domain"))</label>
                        <input id=(format!("domain-{}", tenant.id)) name="domain"
                               autocomplete="off"
                               value=(tenant.domain.clone().unwrap_or_default())>
                        <p class="field-hint">(t("Where its console answers, and the suffix its \
                             service names hang off. It has to point at this node, \
                             and it cannot be one of this node's own names — the \
                             console an operator fixes things from must not be \
                             reachable only through somebody else's."))</p>
                        <div class="actions">
                            <button type="submit">(t("Save domain"))</button>
                        </div>
                    </form>

                    <form method="post" class="stack"
                          action=(format!("/organisations/{}/quota", tenant.id))>
                        <label for=(format!("memory-{}", tenant.id))>(t("Memory"))</label>
                        <input id=(format!("memory-{}", tenant.id)) name="memory"
                               autocomplete="off"
                               placeholder=(t("no ceiling"))
                               value=(tenant.quota_memory.map(crate::node::memory::human).unwrap_or_default())>
                        <label for=(format!("cpu-{}", tenant.id))>(t("CPU"))</label>
                        // **`cores_field`, not `cpu_label`.** One is what a
                        // number is called on a page and the other is what
                        // goes back into the box that produced it — and the
                        // difference bites: `2 cores` read back as `2`, but
                        // a value of `2000 cores` is what somebody typing
                        // millicores into an unlabelled box gets, and it
                        // reads back as two thousand of them.
                        <input id=(format!("cpu-{}", tenant.id)) name="cpu"
                               autocomplete="off"
                               placeholder=(t("no ceiling"))
                               value=(tenant.quota_cpu.map(presets::cores_field).unwrap_or_default())>
                        // The unit, said the way the service's own resources
                        // form says it. Without this the box took "2000" as
                        // two thousand cores, which is what it means and not
                        // what anybody typing it meant.
                        <p class="field-hint">(t("Empty is no ceiling. Memory like 512 MB or 2 GB; \
                             CPU in cores like 0.5 or 2, or millicores with an m, like 500m. \
                             What a service may take is what counts against this, summed \
                             over every copy wherever it runs — the limit is the \
                             reservation, here as everywhere else."))</p>
                        <div class="actions">
                            <button type="submit">(t("Save quota"))</button>
                        </div>
                    </form>

                    // **The invitation lives here, not on the
                    // people page.** It was a selector there for
                    // one afternoon and the form stopped making
                    // sense: the role read "on this node" while
                    // naming an organisation, and the project
                    // list was the node's rather than theirs.
                    // Two of the three questions change meaning
                    // with the scope, and without JavaScript a
                    // list cannot follow a select.
                    //
                    // So the scope is *where the form is*, which
                    // is the shape this console already uses —
                    // placement lives on the service, edges live
                    // on the service. Here the project list is
                    // that organisation's by construction and
                    // the words can say what they mean.
                    <form method="post" class="stack"
                          action=(format!("/organisations/{}/invite", tenant.id))>
                        <p class="card-label">(t("Invite somebody to this organisation"))</p>
                        <label for=(format!("role-{}", tenant.id))>(t("In it"))</label>
                        <select id=(format!("role-{}", tenant.id)) name="node_role">
                            <option value="member">(t("Member — the projects they are added to"))</option>
                            <option value="admin">(t("Administrator — every project of this organisation"))</option>
                        </select>
                        @if held.is_empty() {
                            <p class="field-hint">(t("It has no projects yet. Whoever you invite as \
                                 an administrator can make the first one."))</p>
                        } @else {
                            <label for=(format!("project-{}", tenant.id))>(t("Into a project"))</label>
                            <select id=(format!("project-{}", tenant.id)) name="project">
                                <option value="">(t("Nothing yet"))</option>
                                @for project in held {
                                    <option value=(&project.slug)>(&project.name)</option>
                                }
                            </select>
                            <label for=(format!("project-role-{}", tenant.id))>(t("As"))</label>
                            <select id=(format!("project-role-{}", tenant.id)) name="project_role">
                                @for role in crate::accounts::roles::ProjectRole::ALL {
                                    <option value=(role.as_str())>(word(role.label()))</option>
                                }
                            </select>
                        }
                        <p class="field-hint">(t("A link that works once and expires in seven days. \
                             They choose their own username and password."))</p>
                        // **The link lands on the organisation's
                        // own name**, because that is the door
                        // their account works at — the node's
                        // would sign them up and bounce them on
                        // the first click. Without a domain there
                        // is no such door yet, and saying so
                        // beats handing over a link that appears
                        // to work.
                        @if tenant.domain.is_none() {
                            <p class="field-hint">(t("It has no domain yet, so the link will be on \
                                 this node's name — the account is made, and the console it \
                                 belongs to exists once you give the organisation one."))</p>
                        }
                        <div class="actions">
                            <button type="submit">(t("Create invitation"))</button>
                        </div>
                    </form>

                    <form method="post" action=(format!("/organisations/{}/delete", tenant.id))>
                        <button class="btn btn-ghost destructive" type="submit">
                            (t("Delete organisation"))
                        </button>
                    </form>
                </section>
            }
        </div>
    }
}
