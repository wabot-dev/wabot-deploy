//! Where a name's certificate comes from, and what is installed.

use crate::console::nodes::CertificateCells;
use crate::console::nodes::CertificateState;
use crate::views::layouts::layout;
use crate::views::texts::language::t;
use hypertext::prelude::*;

pub fn certificate_cells(
    state: &CertificateState,
    facts: &crate::console::CertificateFacts,
    domain: Option<&str>,
) -> CertificateCells {
    let (word, badge, dot) = state.badge();
    CertificateCells {
        domain: domain.unwrap_or("not set").to_string(),
        issuer: facts.issuer.clone(),
        // Always a row, even when nothing renews: the script only
        // replaces text that is already on the page, so a row that
        // appears only sometimes is a row that can never update.
        renews: match facts.renews_in_days() {
            Some(days) => format!("{days} days"),
            None => "—".into(),
        },
        word,
        badge,
        dot,
        note: state.note(),
        failure: state.failure().to_string(),
    }
}

/// What certificate this node serves, and how to change it.
///
/// The form is the answer to a node installed before its DNS was
/// ready: it serves its own authority's certificate, and until now
/// there was no way to ask again — with the same name or a different
/// one — without editing a file and restarting.
///
/// The button says what it does. It used to say "Request a
/// certificate", which was one of the things it did, only sometimes,
/// and never the thing an operator came here to do: the field beside
/// it is the node's domain, and saving it is what starts everything
/// else.
pub fn certificate_card<'a>(
    cells: &'a CertificateCells,
    state: &'a CertificateState,
    policy: &'a crate::edge::policy::Policy,
    domain: Option<&'a str>,
    // The two it reads, rather than a page struct: it is rendered from
    // `/node` now, whose query has no `node` in it.
    query: &'a crate::console::nodes::NodeSettingsPage,
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Certificate"))</p>
                <span class=(cells.badge) data-cert="badge">
                    <span class=(cells.dot) data-cert="dot"></span>
                    <span data-cert="word">(cells.word)</span>
                </span>
            </div>

            <dl class="kv">
                <dt>(t("Domain"))</dt>
                <dd data-cert="domain">(&cells.domain)</dd>
                <dt>(t("Issuer"))</dt>
                <dd data-cert="issuer">(&cells.issuer)</dd>
                <dt>(t("Renews in"))</dt>
                <dd data-cert="renews">(&cells.renews)</dd>
            </dl>

            <p class="note" data-cert="note">(cells.note)</p>
            // Always present, empty when there is nothing to say: the
            // script assigns into it, and `.failure:empty` hides it.
            <p class="failure" data-cert="failure">(&cells.failure)</p>

            @if let Some(message) = &query.checked {
                <p class="note">(message)</p>
            }
            @if let Some(message) = &query.error {
                (layout::error_note(message))
            }

            <form method="post" action="/node/certificate" class="stack">
                <label for="domain">(t("Domain"))</label>
                <input id="domain" name="domain" type="text" autocomplete="off" class="mono"
                       value=(domain.unwrap_or_default())
                       placeholder="node.example.com">
                <p class="field-hint">(t("It must resolve to this node, and this node must be reachable on \
                     port 80 — that is what the challenge answers on. Both are checked \
                     before anything is requested. Saving reissues: the node takes the \
                     new name on its own certificate straight away, then asks a public \
                     authority for one."))</p>
                <div class="actions">
                    <button type="submit">(t("Save domain"))</button>
                    @if state.may_retry() {
                        // Asking again with the same name is its own
                        // action: the usual fix for a refusal is out
                        // there — a DNS record, a firewall — and having
                        // to re-type the domain to retry reads as
                        // though the domain were the problem.
                        <button class="btn btn-secondary" type="submit"
                                name="retry" value="1">(t("Try again"))</button>
                    }
                </div>
            </form>

            (certificate_source_form("/node/certificate/source", policy))
        </section>
    }
}

/// Where one name's certificate comes from.
///
/// Shared by the node's own certificate and every service hostname,
/// because it is the same question — the node's domain is not a
/// special case, it is only the name that had a page first. The ids
/// carry the action so two of these on one page do not collide.
/// Read a certificate source out of a form, refusing what cannot work.
///
/// **Shared, because it is the same question wherever it is asked** — the
/// node's own name, a service's hostname, a database's. It was written
/// twice with the same reasoning in both, and a third copy is what made it
/// worth one: the two subtleties here are the file pair being *read* before
/// it is trusted, and anything unrecognised falling back to ACME rather
/// than to a file source that could lose a certificate.
/// What signed it, in three words.
///
/// "Valid" says nothing about who signed it, and a name on a self-signed
/// certificate is a browser warning somebody is about to meet.
pub fn source_word(renew_with: &crate::edge::policy::RenewWith) -> &'static str {
    match renew_with {
        crate::edge::policy::RenewWith::Acme => t("Let's Encrypt"),
        crate::edge::policy::RenewWith::SelfSigned => t("self-signed"),
        crate::edge::policy::RenewWith::File { .. } => t("from a file"),
    }
}

pub fn certificate_source_form(
    action: &str,
    policy: &crate::edge::policy::Policy,
) -> impl Renderable {
    let (cert_path, key_path) = match &policy.renew_with {
        crate::edge::policy::RenewWith::File {
            cert_path,
            key_path,
        } => (cert_path.as_str(), key_path.as_str()),
        _ => ("", ""),
    };
    let field = |suffix: &str| format!("{}-{suffix}", policy.name);

    // An island host, not a bare form: the client runtime re-hydrates
    // registered islands after a boosted navigation, and a listener
    // attached once at load would belong to a form the next swap threw
    // away. Without scripting this is a `<wabot-island>` wrapper around
    // a form that works, which is the whole cost.
    wabot::ui::hypertext::island_bare(
        "fields",
        rsx! {
            <form method="post" action=(action) class="stack">
                <label for=(field("source"))>(t("Where the certificate comes from"))</label>
                <select id=(field("source")) name="renew_with">
                    @for (value, label) in crate::console::nodes::SOURCES {
                        @if policy.renew_with.as_str() == *value {
                            <option value=(value) selected>(label)</option>
                        } @else {
                            <option value=(value)>(label)</option>
                        }
                    }
                </select>

                // Only the file answer needs these, so only the file
                // answer shows them. Without scripting they stay visible
                // and the form still works — the server reads them only
                // when `renew_with` says `file`.
                <div class="stack" data-when="renew_with=file">
                    <label for=(field("cert"))>(t("Certificate file"))</label>
                    <input id=(field("cert")) name="cert_path" type="text" autocomplete="off" class="mono"
                           value=(cert_path) placeholder="/etc/ssl/name.crt"
                           data-required-when="renew_with=file">
                    <label for=(field("key"))>(t("Key file"))</label>
                    <input id=(field("key")) name="key_path" type="text" autocomplete="off" class="mono"
                           value=(key_path) placeholder="/etc/ssl/name.key"
                           data-required-when="renew_with=file">
                    <p class="field-hint">(t("Both are read now and refused if they do not match, do not cover \
                         this name, or have already expired — a bad pair installed would \
                         break every handshake, including the one serving this page. After \
                         that the node rereads them and reinstalls whatever it finds, which \
                         is how a certificate it cannot renew stays current."))</p>
                </div>
                <div class="actions">
                    <button class="btn btn-secondary" type="submit">(t("Save source"))</button>
                </div>
            </form>
        },
    )
}

/// The two cards that update themselves, in one island host.
///
/// Props carry the node id, because the client needs it to open the
/// stream and reading it out of the URL would be the browser deriving
/// something the server already knew.
#[allow(clippy::too_many_arguments)]
/// The certificate, on the page that can change it.
///
/// Its own island since the meters left for the node's status page: the
/// three used to share one host because they were on one page. The
/// runtime tolerates a host that carries only part of the payload's
/// markup — `set(host.querySelector(…))` skips what it cannot find — so
/// splitting them costs nothing but this function.
pub fn certificate_island<'a>(
    node_id: &'a str,
    cells: &'a CertificateCells,
    state: &'a CertificateState,
    policy: &'a crate::edge::policy::Policy,
    domain: Option<&'a str>,
    query: &'a crate::console::nodes::NodeSettingsPage,
) -> impl Renderable + 'a {
    let inner = rsx! {
        (certificate_card(cells, state, policy, domain, query))
    }
    .render()
    .into_inner();

    wabot::ui::hypertext::island(
        "node-live",
        // Asked for by id rather than by a word meaning "this one": the
        // endpoint takes an id, and a second spelling for the same thing
        // is how a stream ends up answering about the wrong machine.
        &serde_json::json!({ "node": node_id }),
        hypertext::Raw::dangerously_create(&inner),
    )
}
