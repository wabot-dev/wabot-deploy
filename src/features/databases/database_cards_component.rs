//! What a managed database's page shows.
//!
//! Cards rather than one block: they fail separately, and an operator
//! reads them one at a time — the name, the certificate, the port, the
//! backup window, and the connection strings.
//!
//! The password is behind a `<details>`: the page is opened to read a
//! state badge far more often than to copy a credential, and a
//! connection string in plain sight is one that ends up in a
//! screenshot. Markup rather than script, so it works with scripting
//! off like everything else here.

use crate::domain::policies::postgres;
use crate::domain::policies::presets;
use crate::features::databases::texts::t;
use hypertext::prelude::*;

use crate::domain::entities::database::Database;

/// What each size actually gives the engine.
///
/// On the form rather than in the documentation, because the number in
/// the selector is the container's ceiling and the numbers that decide
/// whether a query runs are these.
pub fn sizes_table() -> impl Renderable {
    rsx! {
        <table>
            <thead>
                <tr>
                    <th>(t("Memory"))</th>
                    <th>("shared_buffers")</th>
                    <th>("work_mem")</th>
                    <th>(t("Connections"))</th>
                </tr>
            </thead>
            <tbody>
                @for rung in presets::LADDER {
                    <tr>
                        <td>(presets::label(rung))</td>
                        <td class="mono">(format!("{} MB", postgres::tuning(rung).shared_buffers_mb))</td>
                        <td class="mono">(format!("{} MB", postgres::tuning(rung).work_mem_mb))</td>
                        <td class="mono">(postgres::tuning(rung).max_connections.to_string())</td>
                    </tr>
                }
            </tbody>
        </table>
    }
}

/// What the service page shows for a managed database, in place of the
/// image and the environment nobody may edit.
///
/// The password is behind a `<details>`: the page is opened to read a
/// state badge far more often than to copy a credential, and a
/// connection string in plain sight is one that ends up in a
/// screenshot. It is markup rather than script, so it works with
/// scripting off like everything else here.
/// How far back this database can be taken, and how recent it can land.
///
/// Four answers, and three of them say "not what you think" — see
/// `backup::Window`. The one worth building a card for is
/// `NoAnchor`: log arriving with no backup to replay it onto looks
/// exactly like working, and recovers nothing.
/// Go back to a moment — into a copy, never over the original.
///
/// Jorge asked for this: the window card said "any moment between these
/// two" and there was nowhere to go with it. `restore` existed as a
/// command only, so recovering meant an SSH session in the middle of the
/// thing you were recovering from.
///
/// **Only this database.** Not the node's backup applied whole — what
/// this asks for is one database's volume out of one backup, unpacked
/// beside the original, which goes on serving. That is what makes it
/// something an operator can actually press: nothing is taken away, and
/// comparing the two is most of what a recovery is.
///
/// Absent when there is nothing to reach: no backup holding this
/// database means the form would refuse whatever was typed, and a form
/// that cannot succeed is worse than a sentence saying why.
pub fn restore_card<'a>(
    project_slug: &'a str,
    service_slug: &'a str,
    window: &'a crate::infra::backup::Window,
    default_name: &'a str,
    // Where this node sends its backups, when that is not this machine.
    // Named on the card because it is where the copy will come *from*: a
    // node with a schedule and a bucket has nothing here at all — the
    // staging copy is removed the moment the transfer succeeds.
    elsewhere: Option<&'a str>,
) -> impl Renderable + 'a {
    use crate::infra::backup::Window;

    // The end of the window, as a moment somebody can type. It is the
    // last thing a restore can reach — later than that is a request the
    // archive cannot answer — so it is the placeholder rather than a
    // guess at what they want.
    let latest = match window {
        Window::Between { to, .. } => crate::shell::layouts::layout::typed(*to),
        Window::OnlyTheBackup { at } => crate::shell::layouts::layout::typed(*at),
        _ => String::new(),
    };

    rsx! {
        <section class="stack">
            <p class="card-label">(t("Restore into a copy"))</p>
            <form method="post"
                  action=(format!("/projects/{project_slug}/services/{service_slug}/restore"))
                  class="card stack">
                <label for="restore-at">(t("Up to which moment"))</label>
                <input id="restore-at" name="at" type="text" autocomplete="off" class="mono"
                       value="" placeholder=(&latest)>
                <p class="field-hint">(t("UTC, as the window above reads it: 2026-08-16 14:32. \
                     Empty means as far as the archived log goes, which is the most recent \
                     moment there is."))</p>

                <label for="restore-into">(t("Call the copy"))</label>
                <input id="restore-into" name="into" type="text" autocomplete="off" class="mono"
                       value="" placeholder=(default_name)>
                <p class="field-hint">(t("A new database in this project, with the same \
                     credentials and its own name. The original keeps running and keeps its \
                     data — this is a copy beside it, and deleting it later costs nothing."))</p>
                <p class="field-hint">(t("It unpacks the backup and replays the log at its first \
                     deployment, which starts as soon as you press this. A large database takes \
                     as long to come up as it took to copy."))</p>
                @if let Some(destination) = elsewhere {
                    <p class="field-hint">(t("The backup is looked for on this machine first, \
                         then at "))<span class="mono">(destination)</span>
                        (t(" — where a schedule sends them, and where they are the only copy \
                         once the transfer has succeeded. Downloading it is part of that first \
                         deployment, so a large one takes as long as it takes."))</p>
                }

                <div class="actions">
                    <button class="btn btn-secondary" type="submit">(t("Restore into a copy"))</button>
                </div>
            </form>
        </section>
    }
}

/// One string, and how to spell the name it uses.
pub struct Choice {
    /// `full` or `short` — what the radio selects and the stylesheet
    /// matches on.
    which: &'static str,
    label: &'static str,
    dsn: String,
}

/// The four strings this database can be reached by, all four rendered.
///
/// ## Why all four, and why the choosing is CSS
///
/// The page must work with scripting off, so the alternative to rendering
/// every string is a round trip to change a radio button. They are four
/// short lines: the markup holds them all and `:has()` shows the chosen
/// one. Nothing is fetched, nothing is built in the browser, and with no
/// JavaScript at all the first one is showing because its radio is checked.
pub struct Strings {
    /// The long name — `orders.db-test.<domain>` — and the short one,
    /// `orders.db-test`. Both resolve in every container of the project;
    /// the long one also resolves in the world's DNS, which is what makes
    /// it the only form an authority could ever sign.
    primary: Vec<Choice>,
    pool: Vec<Choice>,
}

impl Strings {
    /// Built from the certificate's own name list, so every string offered
    /// is one `verify-full` will accept.
    ///
    /// `None` when the database has no name at all, which is a database that
    /// has never been deployed.
    /// **Only what actually verifies.**
    ///
    /// A page that offered a string `verify-full` rejects would be worse
    /// than one that offered none, and there are two ways to produce one:
    ///
    /// - With a public authority, the short names are not on the
    ///   certificate and never can be — nothing outside this node resolves
    ///   them, so there is no challenge for an authority to set. Offering
    ///   the short form there is offering a string that fails.
    /// - On a node with no domain there **is** no long name: nothing to
    ///   derive one from, so the short one is not a convenience, it is the
    ///   only name the database has.
    ///
    /// So the name is a choice only when both spellings work, which is a
    /// node with a domain whose certificate it signs itself. Otherwise
    /// there is one spelling and no group to show.
    pub fn of(row: &Database, names: &[String], signs_here: bool) -> Option<Self> {
        // `verify-full` always: encryption without identity is what
        // `require` buys, and every name here is on the certificate.
        //
        // The authority only when this node is the one that signed. With a
        // public one the client's own trust store is what checks it, and
        // naming a file that a laptop does not have would be a string that
        // fails for the one reader most likely to paste it.
        let suffix = match signs_here {
            true => "?sslmode=verify-full&sslrootcert=/etc/wabot/ca.crt",
            false => "?sslmode=verify-full",
        };
        let dsn = |host: &str| {
            format!(
                "{}{suffix}",
                postgres::connection_url(
                    &row.admin_user,
                    &row.admin_password,
                    host,
                    postgres::PORT,
                    &row.database_name,
                )
            )
        };
        // Longest first is how `certificate_names` orders them: the
        // qualified name, then the bare slug, then slug and project. So the
        // qualified pair is whatever holds a dot past the project, and the
        // short pair is the two-part one — chosen by shape rather than by
        // rebuilding the names here, which would be a second opinion about
        // what this database is called.
        let pool_mark = crate::infra::deploy::hosts::READ_ONLY;
        let is_pool = |name: &&String| {
            name.split('.')
                .next()
                .is_some_and(|first| first.ends_with(pool_mark))
        };
        let parts = |name: &&String| name.split('.').count();

        let primary_short = names
            .iter()
            .filter(|name| !is_pool(name) && parts(name) == 2)
            .min_by_key(|name| name.len())?
            .clone();
        let pool_short = names
            .iter()
            .filter(|name| is_pool(name) && parts(name) == 2)
            .min_by_key(|name| name.len())?
            .clone();

        let long_of = |pool: bool| {
            names
                .iter()
                .find(|name| is_pool(name) == pool && parts(name) > 2)
                .map(|name| dsn(name))
        };
        let offer = |long: Option<String>, short: String| {
            let mut choices = Vec::new();
            if let Some(long) = long {
                choices.push(Choice {
                    which: "full",
                    label: "Long name",
                    dsn: long,
                });
            }
            // The short one only where it is on the certificate, which is
            // where this node signed it.
            if signs_here || choices.is_empty() {
                choices.push(Choice {
                    which: "short",
                    label: "Short name",
                    dsn: short,
                });
            }
            choices
        };

        Some(Self {
            primary: offer(long_of(false), dsn(&primary_short)),
            pool: offer(long_of(true), dsn(&pool_short)),
        })
    }

    /// Whether the name is a choice at all.
    pub fn names_differ(&self) -> bool {
        self.primary.len() > 1
    }
}

pub fn window_card<'a>(
    window: &'a crate::infra::backup::Window,
    // The destination, when the backups leave this machine. It changes
    // what `NoAnchor` *means*: not "there is no backup", which is what
    // this card used to say, but "there is none here" — and with a bucket
    // configured that is the ordinary, healthy state.
    elsewhere: Option<&'a str>,
) -> impl Renderable + 'a {
    use crate::infra::backup::Window;

    rsx! {
        <section class="card stack" id="recovery">
            <div class="split">
                <p class="card-label">(t("Going back"))</p>
                @match window {
                    Window::NotKeeping => {
                        <span class="badge">(t("Not kept"))</span>
                    }
                    Window::NoAnchor => {
                        <span class="badge badge-danger">
                            <span class="dot dot-danger"></span>(t("Recovers nothing"))
                        </span>
                    }
                    Window::OnlyTheBackup { .. } => {
                        <span class="badge badge-info">
                            <span class="dot dot-info"></span>(t("The backup only"))
                        </span>
                    }
                    Window::Between { gap: true, .. } => {
                        <span class="badge badge-warning">
                            <span class="dot dot-warning"></span>(t("Broken part way"))
                        </span>
                    }
                    Window::Between { .. } => {
                        <span class="badge badge-success">
                            <span class="dot dot-success"></span>(t("Any minute"))
                        </span>
                    }
                }
            </div>

            @match window {
                Window::NotKeeping => {
                    <p class="field-hint">(t("This node is not keeping the write-ahead log, so a \
                         restore reaches the moment of a backup and no further. The switch is \
                         on the node's Backup tab."))</p>
                }
                // The one that looks like it is working.
                Window::NoAnchor => {
                    @if let Some(destination) = elsewhere {
                        <p class="field-hint">(t("The log is being kept and no backup is on this \
                             machine — which is what a destination means: they are sent to "))
                            <span class="mono">(destination)</span>
                            (t(" and the local copy is removed once the transfer succeeds. The \
                             restore below looks there, so this window is the log's reach and \
                             not the whole answer."))</p>
                    } @else {
                        <p class="field-hint">(t("The log is being kept and there is no backup to \
                             replay it onto. A base backup is what the log is applied to, so \
                             until one is taken this recovers nothing at all — the disk is being \
                             spent on something nothing can use. The node's Backup tab is where \
                             one is scheduled, or taken now."))</p>
                    }
                }
                Window::OnlyTheBackup { at } => {
                    <p class="field-hint">
                        (t("A backup from "))<span class="mono">(crate::shell::layouts::layout::exactly(*at))</span>
                        (t(", and no log after it yet. A minute of writes is what produces the \
                             first segment."))
                    </p>
                }
                Window::Between { from, to, gap } => {
                    <p class="field-hint">
                        (t("Any moment between "))<span class="mono">(crate::shell::layouts::layout::exactly(*from))</span>
                        (t(" and "))<span class="mono">(crate::shell::layouts::layout::exactly(*to))</span>
                    </p>
                    // The end of the window is the hole, not the newest
                    // segment: recovery stops there, and everything
                    // archived afterwards is being kept and cannot be
                    // reached.
                    @if *gap {
                        <p class="field-hint">(t("A segment is missing, so the window ends where \
                             it does rather than at the newest thing archived — recovery stops \
                             at the gap. What is kept after it cannot be reached. Taking a \
                             backup now starts a whole window again."))</p>
                    } @else {
                        <p class="field-hint">(t("The later end is the last segment archived, \
                             not the current moment: a segment closes a minute or two after \
                             the one before it."))</p>
                    }
                }
            }
        </section>
    }
}

pub fn database_card<'a>(
    row: &'a Database,
    replicas: &'a [crate::domain::entities::replica::Replica],
    address: Option<String>,
    memory_limit: Option<u64>,
    // Every name it answers to, from `deploy::certificate_names` — the same
    // list the certificate is built from, so this page cannot offer a string
    // naming something the certificate does not cover.
    names: &'a [String],
    // Whether this node is the one signing. It decides a single parameter,
    // and getting it wrong is a string that fails: `sslrootcert` pointing at
    // an authority that did not sign is as broken as leaving it out when it
    // did.
    signs_here: bool,
    // The port the world reaches it on, when there is one. The strings
    // above are the project's own; this is what makes the sentence under
    // them true rather than a description of the product at some past
    // moment.
    published: Option<u16>,
) -> impl Renderable + 'a {
    // Only once it has an address, which is only once it has been
    // deployed. The names exist from the moment the database does — they
    // are derived from the project and the node's domain — so without this
    // the page would hand out a connection string for something that has
    // never run.
    //
    // The address itself is not shown here: every copy's is in the
    // placement table below, which is where somebody looking for one
    // looks.
    let strings = address
        .as_ref()
        .and_then(|_| Strings::of(row, names, signs_here));
    let copies = replicas.len();
    let standbys = replicas
        .iter()
        .filter(|replica| row.role_of(replica.slot) == postgres::Role::Standby)
        .count();

    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Database"))</p>
                <span class="who">
                    (row.engine.label())(" ")(&row.version)
                </span>
            </div>
            <dl class="kv">
                <dt>(t("Database name"))</dt>
                <dd class="mono">(&row.database_name)</dd>
                <dt>(t("User"))</dt>
                <dd class="mono">(&row.admin_user)</dd>
                <dt>(t("Memory"))</dt>
                <dd>(
                    memory_limit
                        .map(presets::label)
                        .unwrap_or_else(|| t("no ceiling").to_string())
                )</dd>
                <dt>(t("Copies"))</dt>
                <dd>(format!("{copies} ({standbys} read-only)"))</dd>
                // The row has always kept both, and no page said either: a
                // restored copy read exactly like a database made empty.
                // Found on a VM node.
                @if let Some(from) = &row.restored_from {
                    <dt>(t("Restored from"))</dt>
                    <dd class="mono">(from)</dd>
                    <dt>(t("Up to"))</dt>
                    <dd>(
                        row.recovery_target
                            .as_deref()
                            .map(|moment| format!("{moment} UTC"))
                            .unwrap_or_else(|| t("as far as the archived log went").to_string())
                    )</dd>
                }
            </dl>
            @if let Some(strings) = &strings {
                (connection_block(strings, published))
            } @else {
                <p class="tile-detail">(t("It has no address yet. A connection string appears once \
                     it has been deployed."))</p>
            }
        </section>
    }
}

/// One string at a time, chosen without a round trip and without a script.
///
/// The string used to be a paragraph, so a browser broke it across lines
/// wherever it liked — and a connection string with a line break in it is a
/// connection string nobody can use. It is a `<pre>` that scrolls now.
///
/// The copy button is **revealed** by the `copy` island rather than
/// rendered: with scripting off a button that cannot copy is a control that
/// lies, and the text is selectable either way.
/// What this database is called.
///
/// **A database always has a name**, unlike a service: a name is how it is
/// reached and how its certificate is verified, and there is no version of
/// this that works without one. So the field is never empty — it starts at
/// what every database had before a name could be chosen,
/// `<service>.<project>.<the node's domain>`, and the operator may put
/// anything there, including a domain that has nothing to do with this
/// machine's.
///
/// The read pool's name is not asked for. It is the primary's with `-ro` in
/// the first label, so one name governs both and there is no second thing
/// to keep in step — see `hosts::pool_name`.
pub fn name_card<'a>(action: &'a str, name: &'a str, pool: &'a str) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack" id="name">
            <p class="card-label">(t("Name"))</p>
            <form method="post" action=(action) class="stack">
                <input id="db-name" name="name" type="text" autocomplete="off"
                       class="mono" value=(name) required>
                <p class="field-hint">
                    (t("The read pool answers at "))<span class="mono">(pool)</span>
                    (t(" — the same name with -ro in its first label. Changing this \
                         reissues the certificate and rewrites the names inside every \
                         container of this project."))
                </p>
                <div class="actions">
                    <button class="btn btn-secondary" type="submit">(t("Save name"))</button>
                </div>
            </form>
        </section>
    }
}

/// Where this database's certificate comes from, and how to verify it.
///
/// The same control a service's hostname has and the node's own — one
/// question asked in three places, so it is one form. What a database adds
/// is the other half: with the node signing, a client outside a container
/// has nothing to check against until it holds the authority, so the page
/// hands it over rather than describing it.
pub fn certificate_card<'a>(
    action: &'a str,
    policy: &'a crate::infra::edge::policy::Policy,
    // What the node holds for this name right now. A source without a state
    // is a page saying what was *asked for* and never what happened —
    // which is what sent Jorge to `doctor` to find out whether his
    // certificate had been issued.
    cells: &'a crate::shell::components::node_certificate::CertificateCells,
    acme: bool,
) -> impl Renderable + 'a {
    let signs_here = matches!(
        policy.renew_with,
        crate::infra::edge::policy::RenewWith::SelfSigned
    );
    rsx! {
        <section class="card stack" id="certificate">
            <div class="split">
                <p class="card-label">(t("Certificate"))</p>
                <span class=(cells.badge)>
                    <span class=(cells.dot)></span>(t(cells.word))
                </span>
            </div>

            // The same three facts a service's name shows, because it is
            // the same question about a different name.
            <dl class="kv">
                <dt>(t("Issuer"))</dt>
                <dd>(&cells.issuer)</dd>
                <dt>(t("Renews in"))</dt>
                <dd>(&cells.renews)</dd>
            </dl>
            @if !cells.failure.is_empty() {
                <p class="failure">(&cells.failure)</p>
            }

            (crate::shell::components::node_certificate::certificate_source_form(action, policy, acme))
            @if signs_here {
                <p class="field-hint">(t("This node signs it, so a client has to be given the \
                     authority before it can verify anything. A container gets it at \
                     /etc/wabot/ca.crt; anything else needs the file."))</p>
                <div class="actions">
                    <a class="btn btn-secondary btn-sm" href="/ca.crt" download>
                        (t("Download the authority"))
                    </a>
                </div>
            } @else {
                <p class="field-hint">(t("A public authority signs it, so any client verifies it \
                     with the trust store it already has — the connection string needs no \
                     certificate of its own."))</p>
            }
        </section>
    }
}

/// Whether the world can reach this database, and on what port.
///
/// **The number is the node's to choose.** Two databases on one machine
/// cannot share a host port, and asking for one would be asking somebody
/// to remember what every other service on the node already took — the
/// allocator picks the lowest free one out of the node's range and the
/// unique index is what makes that safe.
///
/// Separate from the certificate, because they answer different questions
/// and Jorge named the difference: a certificate makes a name *verifiable*
/// and a port makes the database *reachable*. Until this is on, the name
/// resolves from outside and nothing answers.
pub fn published_card<'a>(
    action: &'a str,
    host_port: Option<u16>,
    name: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack" id="published">
            <div class="split">
                <p class="card-label">(t("From outside the node"))</p>
                @if host_port.is_some() {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>(t("Published"))
                    </span>
                } @else {
                    <span class="badge">(t("Not published"))</span>
                }
            </div>

            @if let Some(port) = host_port {
                <dl class="kv">
                    <dt>(t("Reachable at"))</dt>
                    <dd class="mono">(name)(":")(port.to_string())</dd>
                </dl>
                <p class="field-hint">(t("The node chose the port, so it cannot collide with \
                     another database's. It survives a redeployment and changes only if you \
                     turn this off and on again."))</p>
            } @else {
                <p class="field-hint">(t("Nothing outside this node can reach it until it is \
                     published. Publishing opens a port on every interface of this node and \
                     maps it to the primary."))</p>
            }

            <form method="post" action=(action)>
                @if host_port.is_some() {
                    <input type="hidden" name="publish" value="off">
                    <button class="btn btn-ghost destructive" type="submit">
                        (t("Stop publishing"))
                    </button>
                } @else {
                    <input type="hidden" name="publish" value="on">
                    <button class="btn btn-secondary" type="submit">(t("Publish"))</button>
                }
            </form>
        </section>
    }
}

/// One string, in a block that does not break it.
pub fn value<'a>(
    which: &'a str,
    dsn: &'a str,
    copy: &'a str,
    copied: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        // The line is what CSS shows or hides, so the button the island
        // appends belongs to the string beside it rather than to the three
        // that are not on screen — and it sits next to the string instead
        // of on top of the end somebody is reading.
        <div class="dsn-line" data-dsn=(which)>
            <pre class="dsn-value" data-copy
                 data-copy-label=(copy) data-copied-label=(copied)>(dsn)</pre>
        </div>
    }
}

pub fn connection_block(strings: &Strings, published: Option<u16>) -> impl Renderable + '_ {
    // The words the island puts on the button it makes. Passed as data
    // because the script has no language and the account has one — the same
    // rule the badge words follow.
    let (copy, copied) = (t("Copy"), t("Copied"));
    let inner = rsx! {
        <details>
            <summary>(t("Connection string"))</summary>
            <div class="dsn stack-sm">
                // Two groups when there are two questions, one when the
                // name is not a choice — a node with no domain has only
                // the short name, and a public authority can only ever
                // sign the long one.
                <div class="dsn-pick">
                    @if strings.names_differ() {
                        <div class="dsn-group">
                            @for (index, choice) in strings.primary.iter().enumerate() {
                                <label class="check">
                                    @if index == 0 {
                                        <input type="radio" name="dsn-name"
                                               id=(format!("dsn-{}", choice.which)) checked>
                                    } @else {
                                        <input type="radio" name="dsn-name"
                                               id=(format!("dsn-{}", choice.which))>
                                    }
                                    (t(choice.label))
                                </label>
                            }
                        </div>
                    }
                    <div class="dsn-group">
                        <label class="check">
                            <input type="radio" name="dsn-target" id="dsn-primary" checked>
                            (t("Primary"))
                        </label>
                        <label class="check">
                            <input type="radio" name="dsn-target" id="dsn-pool">
                            (t("Read pool"))
                        </label>
                    </div>
                </div>

                // Every string that verifies, and the stylesheet shows the
                // chosen one. With no name group there is one spelling, so
                // its radio is the only thing selecting.
                <div class="dsn-values">
                    @for choice in &strings.primary {
                        (value(&format!("primary-{}", choice.which), &choice.dsn, copy, copied))
                    }
                    @for choice in &strings.pool {
                        (value(&format!("pool-{}", choice.which), &choice.dsn, copy, copied))
                    }
                </div>

                // The one line that stops a name being read as a promise —
                // and it has to know whether the port is open, because it
                // said "publishing is not built" for as long as that was
                // true and went on saying it after Jorge published one.
                // A hint that describes the product rather than this
                // database is a hint that goes stale on its own.
                @if let Some(port) = published {
                    <p class="field-hint">
                        (t("These reach it from inside the project, on any node holding a \
                             copy. From outside the node it answers on the published port, \
                             "))<span class="mono">(port.to_string())</span>(t(" — same name, \
                             same certificate."))
                    </p>
                } @else {
                    <p class="field-hint">(t("These reach it from inside the project, on any \
                         node holding a copy. From outside, nothing answers until a port is \
                         published below."))</p>
                }
            </div>
        </details>
    }
    .render()
    .into_inner();

    // Rendered first, then wrapped: `rsx!` expands to a closure that
    // captures by move, and nesting one inside the island's would have both
    // wanting the same borrows.
    wabot::ui::hypertext::island(
        "copy",
        &serde_json::json!({}),
        hypertext::Raw::dangerously_create(&inner),
    )
}
