//! Where a name's certificate comes from, and what is installed.

use crate::web::views::texts::language::t;
use hypertext::prelude::*;

/// The three answers, in the order somebody is likeliest to want them.
pub const SOURCES: &[(&str, &str)] = &[
    ("acme", "A public authority (Let's Encrypt)"),
    ("self_signed", "This node's own authority"),
    ("file", "Read from files on this node"),
];

/// What the node's TLS is, as one answer.
///
/// Assembled rather than read off a single column, because the honest
/// answer needs three sources: whether a public authority is being
/// asked at all, what is installed, and whether the loop is asking
/// right now. The page used to show only the last *failure*, so a node
/// that had just been given a domain looked identical to one that had
/// never been asked about — and the button said "Request a
/// certificate" whether or not a request was possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertificateState {
    /// ACME is off in the config. The local authority is the answer,
    /// not a stage on the way to one.
    Local,
    /// No domain, so there is no name a public authority could certify.
    Nameless,
    /// The loop is asking, now.
    Working,
    /// A domain is set, nothing has failed, and nothing has arrived.
    Waiting,
    Trusted,
    /// Real ACME against Let's Encrypt staging: untrusted by design.
    Staging,
    /// Read off disk. Whether a browser trusts it depends on a trust
    /// store this node cannot see, so it is reported as what it is
    /// rather than as trusted or not.
    FromFile,
    /// Asked and refused. The loop keeps trying.
    Failed(String),
}
impl CertificateState {
    pub fn read(
        facts: &crate::web::dtos::certificate_facts::CertificateFacts,
        domain: Option<&str>,
        last_error: Option<String>,
        phase: crate::infra::edge::acme::Phase,
        acme_disabled: bool,
    ) -> Self {
        if acme_disabled {
            return Self::Local;
        }
        if domain.is_none() {
            return Self::Nameless;
        }
        // Before the outcome, on purpose: a renewal of a certificate
        // that is currently fine is still something happening, and a
        // page that hid it would be a page that never shows this state
        // on the node where it works.
        if phase == crate::infra::edge::acme::Phase::Working {
            return Self::Working;
        }
        // Before the trust check, because trust is not a thing this
        // node can decide about a certificate it was handed.
        if facts.source == crate::infra::edge::certs::Source::File {
            return Self::FromFile;
        }
        if facts.trusted {
            return Self::Trusted;
        }
        if facts.issuer == "letsencrypt-staging" {
            return Self::Staging;
        }
        match last_error {
            Some(reason) => Self::Failed(reason),
            None => Self::Waiting,
        }
    }

    /// The word, and the two classes that colour it.
    pub(crate) fn badge(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Local => ("Local", "badge badge-info", "dot dot-info"),
            Self::Nameless => ("Self-signed", "badge badge-info", "dot dot-info"),
            Self::Working => ("Requesting", "badge badge-info", "dot dot-info dot-pulse"),
            Self::Waiting => ("Waiting", "badge badge-info", "dot dot-info dot-pulse"),
            Self::Trusted => ("Trusted", "badge badge-success", "dot dot-success"),
            Self::Staging => ("Not trusted", "badge badge-warning", "dot dot-warning"),
            Self::FromFile => ("From file", "badge badge-info", "dot dot-info"),
            Self::Failed(_) => ("Failed", "badge badge-danger", "dot dot-danger"),
        }
    }

    /// What it means, in one sentence somebody can act on.
    pub(crate) fn note(&self) -> &'static str {
        match self {
            Self::Local => {
                "A public authority is not being asked for this node. It serves its \
                 own authority's certificate, which browsers will not trust."
            }
            Self::Nameless => {
                "This node has no domain, so there is no name an authority could \
                 certify. It serves its own authority's certificate. Set a domain \
                 below and it will ask for a public one."
            }
            Self::Working => {
                "Asking the certificate authority now. This page updates itself when \
                 the answer arrives."
            }
            Self::Waiting => {
                "The domain is set and the request has not been made yet. This page \
                 updates itself when it is."
            }
            Self::Trusted => {
                "This page reached you over TLS with a publicly trusted certificate \
                 the node obtained and installed on its own."
            }
            Self::Staging => {
                "Staging certificate — browsers will not trust it, which is expected. \
                 Set acme.directory to production when you are done testing."
            }
            Self::FromFile => {
                "Read from the files below. This node cannot renew it — it did not \
                 ask for it — so whatever keeps those files current is what keeps \
                 this certificate current. If they stop being refreshed, the node \
                 signs for this name itself rather than let it expire."
            }
            Self::Failed(_) => {
                "The last attempt was refused. The node keeps trying, waiting longer \
                 after each failure, and serves its own authority's certificate in \
                 the meantime."
            }
        }
    }

    pub(crate) fn failure(&self) -> &str {
        match self {
            Self::Failed(reason) => reason,
            _ => "",
        }
    }

    /// Whether asking again, without changing anything, could help.
    pub(crate) fn may_retry(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Waiting)
    }
}
/// Everything the card shows, formatted.
///
/// Built here rather than in the browser so the two halves cannot
/// disagree: the same values render the first paint and every update
/// after it. Same reason as [`Cells`].
#[derive(serde::Serialize)]
pub struct CertificateCells {
    pub domain: String,
    pub issuer: String,
    pub renews: String,
    pub word: &'static str,
    pub badge: &'static str,
    pub dot: &'static str,
    pub note: &'static str,
    pub failure: String,
}

pub fn certificate_cells(
    state: &CertificateState,
    facts: &crate::web::dtos::certificate_facts::CertificateFacts,
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
    policy: &'a crate::infra::edge::policy::Policy,
    domain: Option<&'a str>,
    // The two it reads, rather than a page struct: it is rendered from
    // `/node` now, whose query has no `node` in it.
    // What the DNS check said, which belongs beside the form that asked
    // for it. **Not the page's error** — that is shown once at the top,
    // and this card used to show it a second time.
    checked: Option<&'a str>,
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

            @if let Some(message) = checked {
                <p class="note">(message)</p>
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
pub fn source_word(renew_with: &crate::infra::edge::policy::RenewWith) -> &'static str {
    match renew_with {
        crate::infra::edge::policy::RenewWith::Acme => t("Let's Encrypt"),
        crate::infra::edge::policy::RenewWith::SelfSigned => t("self-signed"),
        crate::infra::edge::policy::RenewWith::File { .. } => t("from a file"),
    }
}

pub fn certificate_source_form(
    action: &str,
    policy: &crate::infra::edge::policy::Policy,
) -> impl Renderable {
    let (cert_path, key_path) = match &policy.renew_with {
        crate::infra::edge::policy::RenewWith::File {
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
                    @for (value, label) in SOURCES {
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
    policy: &'a crate::infra::edge::policy::Policy,
    domain: Option<&'a str>,
    // What the DNS check said, which belongs beside the form that asked
    // for it. **Not the page's error** — that is shown once at the top,
    // and this card used to show it a second time.
    checked: Option<&'a str>,
) -> impl Renderable + 'a {
    let inner = rsx! {
        (certificate_card(cells, state, policy, domain, checked))
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
