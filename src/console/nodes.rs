//! Nodes: the list, and what one is spending.
//!
//! ## The only JavaScript in the product
//!
//! Two things on this page change without anybody pressing anything.
//! Memory changes every second, and a page showing a number from when
//! it was rendered is a page somebody reloads to check. A certificate
//! request finishes minutes after it starts, and the page used to
//! handle that by telling the operator to reload in a few seconds —
//! which is the console admitting it does not know.
//!
//! So this one view carries a script: an `EventSource` that replaces
//! both in place. One stream, not two, because one page holding two
//! connections open is twice the thing to get wrong; memory rides a
//! two-second tick and the certificate rides a change signal from the
//! renewal loop, and `live_stream` merges them.
//!
//! It is deliberately the *whole* of the client's involvement. The
//! page renders complete and correct with scripting off — the script
//! only replaces text and classes that are already there — so the
//! console keeps working on a machine where the stream cannot be
//! opened, which is the machine somebody is most likely looking at
//! this page from. Without it the page is a snapshot, which is what it
//! always was.

use std::collections::BTreeMap;
use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use crate::network::{self};
use crate::node::memory::{self, Snapshot};
use crate::policies::access;

use super::auth::{signed_in, SessionMiddleware};
use super::ConsoleState;
use crate::views::components::node_certificate::{CertificateCells, CertificateState};
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};
use crate::views::pages::node_user_node_settings_page::{NodeTab, Panel};

#[derive(Debug, Deserialize, Validate)]
pub struct NodePage {
    pub node: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Validate, Default)]
pub struct NodeSettingsPage {
    pub error: Option<String>,
    pub checked: Option<String>,
    /// Which tab, from the path. Absent on the bare `/node`, which sends
    /// you to the first one.
    pub tab: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct NodesPage {
    pub error: Option<String>,
    /// Names a join token this node is holding for one read. Not the
    /// token — see `ConsoleState::reveals`.
    pub shown: Option<String>,
    /// The node this one has just joined. A name, not a secret, so the
    /// query string is the right place for it — the same rule
    /// `back_with_error` follows.
    pub joined: Option<String>,
    /// Names a pasted token waiting to be read *once*, so its terms can
    /// be shown before it is spent. Same store and same rule as
    /// `shown`: the nonce travels, the token does not.
    pub review: Option<String>,
}

#[injectable]
pub struct NodePages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl NodePages {
    /// Where the gear lands.
    ///
    /// A redirect rather than a page of its own: a landing page whose
    /// whole content is links to the three pages already in the sidebar
    /// is a click somebody has to make before doing anything. The nodes
    /// are what this section is mostly about, so that is where it opens
    /// — and the sidebar says what else is here.
    #[view("/settings")]
    #[middleware(SessionMiddleware)]
    ///
    /// **This is the machine's gear, and it only ever opens the
    /// machine's settings.** Another frontier's console is a different
    /// router with its own landing — see `console::tenant` — so this
    /// controller has no second case to carry, which is what keeps the
    /// two halves separable.
    async fn settings(&self) -> UiResult<ViewOutcome> {
        match signed_in(&self.auth)
            .is_some_and(|account| crate::authz::node_user::manages_the_machine(&account))
        {
            true => Ok(Redirect::found("/node").into()),
            false => Ok(Redirect::found("/").into()),
        }
    }

    /// Where `/nodes` used to be, both of them.
    ///
    /// The list moved to its own area and the machine's own settings to
    /// `/node`, so the old path cannot land on either without choosing.
    /// It chooses the list: that is what the plural said, and a link kept
    /// from before is far more likely to be somebody's bookmark of "how
    /// are my nodes" than of the certificate form.
    #[view("/nodes")]
    #[middleware(SessionMiddleware)]
    async fn nodes_moved(&self) -> UiResult<ViewOutcome> {
        Ok(Redirect::found("/network").into())
    }

    #[view("/nodes/:node")]
    #[middleware(SessionMiddleware)]
    async fn node_moved(&self, query: NodePage) -> UiResult<ViewOutcome> {
        Ok(Redirect::found(format!("/network/{}", query.node)).into())
    }

    #[view("/network")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        // The node belongs to whoever runs it. Hiding the link is
        // courtesy; this is the boundary.
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        let nodes = network::all(&self.state.database).await?;
        let authorities = network::authorities(&self.state.database).await?;
        // What each of them was actually agreed to, which is the whole
        // point of the terms screen: a row saying only "takes
        // instructions from" would hide the difference between a node
        // that may do everything here and one that may do nothing.
        let mut agreed: Vec<(String, Vec<network::capability::Capability>)> = Vec::new();
        for authority in &authorities {
            agreed.push((
                authority.node_id.clone(),
                network::capability::granted_to(&self.state.database, &authority.node_id).await,
            ));
        }
        let projects = access::projects_for(&self.state.database, &account).await?;
        // A token pasted a moment ago, held for exactly this render so
        // its terms can be read before it is spent. Taken, not copied:
        // a reload asks for it again, which is the same behaviour the
        // minted token has and is the honest one for a secret.
        let reviewing = query
            .review
            .as_deref()
            .and_then(|nonce| self.state.reveals.take(nonce))
            .and_then(|token| {
                network::token::JoinToken::decode(&token)
                    .ok()
                    .map(|terms| (token, terms))
            });

        layout::head("Network");
        let frame = Frame::new(&account, Area::Network, &projects, None, "/network");
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::views::pages::node_user_network_page::Network {
            nodes: &nodes,
            authorities: &authorities,
            agreed: &agreed,
            reviewing: reviewing.as_ref(),
            error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_network_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// Mint a token for a node that is not here yet.
    ///
    /// Its own page because it is the consequential half of the network
    /// area: a form that hands out an authority over this machine has no
    /// business half-way down a list somebody opened to check on things.
    /// The token it produces is shown here, once.
    #[view("/network/invite")]
    #[middleware(SessionMiddleware)]
    async fn invite(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        let nodes = network::all(&self.state.database).await?;
        let enrolments = network::enrolment::all(&self.state.database).await?;
        let projects = access::projects_for(&self.state.database, &account).await?;
        // Spent on read: the token exists in clear for exactly this one
        // page load, and a query parameter would put it in the address
        // bar, the history and every refresh. See `ConsoleState::reveals`.
        let revealed = query
            .shown
            .as_deref()
            .and_then(|nonce| self.state.reveals.take(nonce));
        let may_enrol = nodes.iter().any(|node| node.is_self && node.may_be_edge());
        let now = super::now_ms();

        layout::head("Invite a node");
        let frame = Frame::new(&account, Area::Network, &projects, None, "/network/invite");
        let page = crate::views::pages::node_user_invite_node_page::InviteNode {
            nodes: &nodes,
            enrolments: &enrolments,
            revealed: revealed.as_deref(),
            may_enrol,
            now,
            error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_invite_node_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// Spend a token minted somewhere else.
    ///
    /// The terms screen lands here too: a token pasted is held for exactly
    /// one render so what it asks for can be read *before* it is spent.
    /// That order is the whole point — terms shown after committing are a
    /// consent screen for a decision already made.
    #[view("/network/join")]
    #[middleware(SessionMiddleware)]
    async fn join_page(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        let projects = access::projects_for(&self.state.database, &account).await?;
        let reviewing = query
            .review
            .as_deref()
            .and_then(|nonce| self.state.reveals.take(nonce))
            .and_then(|token| {
                network::token::JoinToken::decode(&token)
                    .ok()
                    .map(|terms| (token, terms))
            });

        layout::head("Join a network");
        let frame = Frame::new(&account, Area::Network, &projects, None, "/network/join");
        let page = crate::views::pages::node_user_join_network_page::JoinNetwork {
            reviewing: reviewing.as_ref(),
            joined: query.joined.as_deref(),
            error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_join_network_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// This machine, and only what is editable about it.
    ///
    /// It was `/nodes`, mixed in with the list of every node and the two
    /// enrolment forms — so "what does this node do" and "what are my
    /// nodes doing" were one page with one name. Singular here, and the
    /// list is its own area.
    /// The bare path is not a page: it names the first tab.
    #[view("/node")]
    #[middleware(SessionMiddleware)]
    async fn this_node_root(&self) -> UiResult<ViewOutcome> {
        // Nobody signed in goes to sign in, not to the projects list: the
        // three answers are different and `the_nodes_pages_need_a_session`
        // asks every page in the area which one it gives.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        match account.is_admin() {
            true => Ok(Redirect::found(NodeTab::Domain.path()).into()),
            false => Ok(Redirect::found("/").into()),
        }
    }

    #[view("/node/:tab")]
    #[middleware(SessionMiddleware)]
    async fn this_node(&self, query: NodeSettingsPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        // A word that is not a tab goes to the first one, which covers a
        // hand-typed URL and a link kept from before there were tabs.
        let Some(tab) = query.tab.as_deref().and_then(NodeTab::parse) else {
            return Ok(Redirect::found(NodeTab::Domain.path()).into());
        };

        let projects = access::projects_for(&self.state.database, &account).await?;
        let nodes = network::all(&self.state.database).await?;
        let self_id = nodes
            .iter()
            .find(|node| node.is_self)
            .map(|node| node.id.clone())
            .unwrap_or_default();

        // **Only what this tab draws.** All of it used to be read on
        // every render, and the cost is not theoretical: `backup::taken`
        // is a `read_dir` and a `manifest.json` per backup in the root,
        // and this is the page somebody opens because their node is
        // unwell. The comment saying so was already on one of them.
        let panel = match tab {
            NodeTab::Domain => {
                let domain =
                    crate::node::settings::domain(&self.state.database, &self.state.config).await;
                let facts = super::certificate_facts(&self.state).await;
                let policy = crate::edge::policy::for_name(
                    &self.state.database,
                    &self.state.config,
                    domain
                        .as_deref()
                        .unwrap_or(crate::edge::certs::FALLBACK_NAME),
                )
                .await;
                let state = CertificateState::read(
                    &facts,
                    domain.as_deref(),
                    // This name's own, falling back to the node-wide
                    // reason for a failure recorded before there were
                    // per-name ones.
                    policy
                        .last_error
                        .clone()
                        .or(crate::node::settings::acme_error(&self.state.database).await),
                    self.state.certificates.phase(),
                    self.state.config.acme.disabled,
                );
                let cells = crate::views::components::node_certificate::certificate_cells(
                    &state,
                    &facts,
                    domain.as_deref(),
                );
                Panel::Domain {
                    self_id,
                    cells,
                    state,
                    policy,
                    domain,
                    checked: query.checked.clone(),
                }
            }
            NodeTab::Allows => {
                let domain =
                    crate::node::settings::domain(&self.state.database, &self.state.config).await;
                let mut provides = Vec::new();
                for capability in network::capability::Capability::ALL {
                    let on = network::capability::provides(&self.state.database, capability).await;
                    provides.push((capability, on));
                }
                Panel::Allows {
                    provides,
                    reachable: nodes
                        .iter()
                        .any(|node| node.is_self && node.endpoint.is_some())
                        || domain.is_some(),
                    owns: network::capability::owns(&self.state.database).await,
                    owned: projects.iter().filter(|project| project.is_ours()).count(),
                }
            }
            NodeTab::Resources => Panel::Resources {
                // The reading, not a stream: this page does not host the
                // meters.
                snapshot: self.state.deployer.memory().await,
                reserves: (
                    crate::node::settings::memory_reserve(&self.state.database).await,
                    crate::node::settings::cpu_reserve(&self.state.database).await,
                ),
                cores: crate::node::cpu::cores(),
            },
            NodeTab::Backup => Panel::Backup {
                plan: crate::node::backups::plan(&self.state.database).await,
                attempt: crate::node::backups::last(&self.state.database).await,
                taken: crate::commands::backup::taken(&self.state.config.node.data_dir),
                s3: self.state.config.s3(),
                archiving: crate::node::settings::archiving(&self.state.database).await,
                default_destination: self
                    .state
                    .config
                    .node
                    .data_dir
                    .join("backups")
                    .display()
                    .to_string(),
                config_path: self
                    .state
                    .config
                    .source
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from(crate::config::DEFAULT_CONFIG_PATH))
                    .display()
                    .to_string(),
                self_id,
                // A flag rather than a row, so the card stays a function
                // of values — see `node::backups::in_progress`.
                running: crate::node::backups::in_progress(),
            },
        };

        layout::head("Node");
        let frame = Frame::new(&account, Area::Settings, &projects, None, "/node");
        let page = crate::views::pages::node_user_node_settings_page::NodeSettings {
            tab,
            panel,
            error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_node_settings_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    #[view("/network/:node")]
    #[middleware(SessionMiddleware)]
    async fn node(&self, query: NodePage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }
        let Some(node) = network::find(&self.state.database, &query.node).await? else {
            return Ok(Redirect::found("/network").into());
        };

        let projects = access::projects_for(&self.state.database, &account).await?;

        // A node that is not this one has no memory reading and no
        // certificate: both are answers only the machine itself can
        // give, and nothing asks it anything yet. So the page says what
        // is known rather than rendering cards full of "unknown".
        if !node.is_self {
            let path = format!("/network/{}", node.id);
            // Everything this node could ask that one to run, named the
            // way somebody picking from a list would recognise.
            let orders: Vec<network::errand::Record> = network::errand::all(&self.state.database)
                .await?
                .into_iter()
                .filter(|record| record.node_id == node.id)
                .collect();
            // What this node has agreed to do for that one. Every
            // capability, so the form cannot be missing one — the same
            // rule the capabilities card follows, for the same reason.
            let held = network::capability::granted_to(&self.state.database, &node.id).await;
            let granted: Vec<(network::capability::Capability, bool)> =
                network::capability::Capability::ALL
                    .into_iter()
                    .map(|capability| (capability, held.contains(&capability)))
                    .collect();
            // The releases somebody can ask that node for.
            //
            // **Only when it has agreed to be kept up to date.** Reading
            // the list is a call to GitHub — cached for fifteen minutes,
            // and still a call — and a page that cannot offer the form
            // has no use for it. A failure is a card with an empty list
            // and a sentence saying so, never a page that does not
            // render: this console is opened when a node is unwell, and
            // GitHub being unreachable is one of the ways that happens.
            let may_manage = node
                .allows
                .contains(&network::capability::Capability::Manage);
            let (releases, catalogue_failed) = match may_manage {
                false => (Vec::new(), false),
                true => match self.state.catalogue.releases().await {
                    Ok(releases) => (
                        releases
                            .into_iter()
                            // Not filtered by what *this* machine can
                            // install: the answer belongs to the node
                            // being asked, which resolves the tag itself.
                            // One artifact per release today makes the
                            // two the same; the principle is what keeps
                            // it honest when they stop being.
                            .map(|release| release.tag)
                            .collect(),
                        false,
                    ),
                    Err(error) => {
                        tracing::warn!(%error, "could not read the release list");
                        (Vec::new(), true)
                    }
                },
            };
            layout::head(&node.name);
            let frame = Frame::new(&account, Area::Network, &projects, None, path);
            let page = crate::views::pages::node_user_remote_node_page::RemoteNode {
                node: &node,
                orders: &orders,
                granted: &granted,
                releases: &releases,
                catalogue_failed,
                error: query.error.as_deref(),
            };
            // The account's language, around the render and no wider:
            // the strings are read here, and nothing awaits inside.
            let body = crate::views::texts::language::scoped(account.language, || {
                crate::views::pages::node_user_remote_node_page::render(&page)
                    .render()
                    .into_inner()
            });

            return Ok(frame.render(body).into_view().into());
        }

        let snapshot = self.state.deployer.memory().await;
        // One call, and about the directory the node writes into rather
        // than about `/`: they are usually the same filesystem and the day
        // they are not is the day this matters.
        // Every part of it, which is three directory walks — so once per
        // render and never on the stream beside it: the memory tick is
        // every two seconds, and walking containerd's tree that often
        // would be this node's own reading of itself, at the top of its
        // own CPU column.
        let committed = self.state.deployer.committed().await;
        let reserves = (
            crate::node::settings::memory_reserve(&self.state.database).await,
            crate::node::settings::cpu_reserve(&self.state.database).await,
        );
        let disk = crate::node::disk::breakdown(
            &self.state.config.node.data_dir,
            std::path::Path::new(crate::node::disk::CONTAINERD_ROOT),
        );
        let path = format!("/network/{}", node.id);

        layout::head(&node.name);
        let frame = Frame::new(&account, Area::Network, &projects, None, path);
        let page = crate::views::pages::node_user_this_node_page::ThisNode {
            node: &node,
            disk: &disk,
            committed: &committed,
            snapshot: &snapshot,
            reserves,
        };
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_this_node_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}

/// Where this node's own page is.
///
/// Looked up rather than named by a constant: the id is minted when the
/// node is installed, so there is no path a `const` could hold. Back to
/// the list if there is somehow no row — a redirect to a page that does
/// not exist is worse than one to the page above it.
async fn self_path(database: &wabot::sqlite::SqliteDatabase) -> String {
    match network::me(database).await {
        Ok(Some(me)) => format!("/network/{}", me.id),
        Ok(None) => "/network".to_string(),
        Err(error) => {
            tracing::warn!(%error, "could not read this node's own row");
            "/network".to_string()
        }
    }
}

pub(crate) fn source_from(
    form: &std::collections::HashMap<String, String>,
    name: &str,
) -> Result<crate::edge::policy::RenewWith, String> {
    match super::auth::field(form, "renew_with") {
        "self_signed" => Ok(crate::edge::policy::RenewWith::SelfSigned),
        "file" => {
            let cert_path = super::auth::field(form, "cert_path").trim().to_string();
            let key_path = super::auth::field(form, "key_path").trim().to_string();
            if cert_path.is_empty() || key_path.is_empty() {
                return Err("reading from files needs both a certificate and a key".into());
            }
            // Refused now, while there is somebody to tell.
            crate::edge::certs::from_files(name, &cert_path, &key_path)
                .map_err(|error| error.to_string())?;
            Ok(crate::edge::policy::RenewWith::File {
                cert_path,
                key_path,
            })
        }
        // Including anything a hand-written form might send: ACME is the
        // default, and defaulting to it cannot lose a certificate the way
        // defaulting to a file source could.
        _ => Ok(crate::edge::policy::RenewWith::Acme),
    }
}

/// Store it — **forgetting** rather than writing the default.
///
/// A stored default would go stale the day `acme.disabled` changed, and
/// then say the opposite of what the node does.
pub(crate) async fn store_source(
    database: &wabot::sqlite::SqliteDatabase,
    config: &crate::config::Config,
    name: &str,
    renew_with: &crate::edge::policy::RenewWith,
) -> Result<(), String> {
    let stored = match renew_with == &crate::edge::policy::RenewWith::default_for(config) {
        true => crate::edge::policy::clear(database, name).await,
        false => crate::edge::policy::set(database, name, renew_with).await,
    };
    stored.map_err(|error| error.to_string())
}

/// The three answers, in the order somebody is likeliest to want them.
pub(crate) const SOURCES: &[(&str, &str)] = &[
    ("acme", "A public authority (Let's Encrypt)"),
    ("self_signed", "This node's own authority"),
    ("file", "Read from files on this node"),
];

/// The server half.
#[injectable]
pub struct NodeApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl NodeApi {
    /// The node's certificate authority, as a file.
    ///
    /// Because a client that cannot verify is a client using `require`,
    /// which is encryption without identity. A container is handed this at
    /// `/etc/wabot/ca.crt`; anything else — a laptop, a CI runner, a
    /// machine elsewhere — has nowhere to get it from, and telling somebody
    /// to `psql` their way into a container to copy a file is not an
    /// answer.
    ///
    /// It is the authority's **public** half, which is what an authority is
    /// for. Behind a session anyway: what it discloses is that this node
    /// exists and signs things, and that is nobody's business but the
    /// operator's.
    #[get("/ca.crt")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn authority(&self, _request: Request) -> RestResult<Response> {
        if signed_in(&self.auth).is_none() {
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }
        let pem = match crate::edge::certs::ca_certificate_pem(&self.state.database).await {
            Ok(pem) => pem,
            Err(error) => {
                tracing::warn!(%error, "could not read this node's authority");
                return Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .expect("a constant response is well-formed"));
            }
        };

        Ok(Response::builder()
            .header(header::CONTENT_TYPE, "application/x-pem-file")
            // Named, because a file called `ca.crt` in a downloads folder
            // beside four others is a file nobody can place.
            .header(
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"wabot-node-ca.crt\"",
            )
            .body(Body::from(pem))
            .expect("a constant response is well-formed"))
    }

    #[post("/node/certificate")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_certificate(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let here = &self_path(&self.state.database).await;

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        // "Try again" is the same form with nothing changed. It exists
        // because the usual fix for a refusal is somewhere else — a DNS
        // record, a firewall — and making somebody re-type the domain to
        // retry reads as though the domain were what was wrong.
        if form.contains_key("retry") {
            self.state.certificates.now();
            return Ok(super::auth::see_other(here));
        }

        let settled = crate::usecases::node_user_set_node_domain::run(
            &self.state.database,
            &self.state.config,
            &self.state.deployer,
            &account,
            super::auth::field(&form, "domain"),
        )
        .await;

        // The loop owns issuance — both the local certificate carrying
        // the new name and the public one. Asking it here is what makes
        // saving a domain reissue rather than merely record.
        match settled {
            Ok(crate::dtos::responses::domain_settled::Settled::Named(domain)) => {
                self.state.certificates.now();
                Ok(super::auth::see_other(&format!(
                    "{here}?{}",
                    form_urlencoded::Serializer::new(String::new())
                        .append_pair("checked", &format!("{domain} resolves here."))
                        .finish()
                )))
            }
            Ok(crate::dtos::responses::domain_settled::Settled::Cleared) => {
                self.state.certificates.now();
                Ok(super::auth::back_with_error(
                    here,
                    "the domain was cleared — this node serves its own certificate",
                ))
            }
            Err(reason) => Ok(super::auth::back_with_error(here, &reason)),
        }
    }

    /// Say what this node is willing to do, for anybody at all.
    ///
    /// Not "for these other nodes" — that is a grant, and it is decided
    /// when two nodes agree to know each other. This is one layer above
    /// and it binds this node too: a machine small enough to prefer
    /// running nothing places every copy of its own services elsewhere,
    /// and a machine with a perfectly good address can decline to answer
    /// for names.
    ///
    /// Turning `edge` off makes this node **private**, which is not a
    /// separate switch and must not become one: private has never meant
    /// anything except "does not offer to answer for names".
    /// What this machine keeps for itself.
    ///
    /// **The floor is a measurement, not a constant.** A reserve under
    /// what this node's own processes are using right now — this binary,
    /// containerd, the shims — is refused with the reading, because the
    /// node can answer that question about itself and 256 MB cannot. The
    /// old constant was chosen for exactly this reason and had to guess.
    ///
    /// And a reserve that leaves less than is already promised is refused
    /// too, naming the figure: the ceilings on existing services are
    /// reservations, so accepting it would make every "does this fit"
    /// answer on the node wrong until somebody deleted something.
    #[post("/node/reserves")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_reserves(&self, request: Request) -> RestResult<Response> {
        // The tab this form is on, not `/node` — which is a redirect to
        // the first tab, so a refused reserve carried its message to the
        // domain page and a saved one landed two tabs from the number it
        // had just changed.
        let here = &NodeTab::Resources.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        if let Err(reason) = crate::usecases::node_user_set_reserves::run(
            &self.state.database,
            &self.state.deployer,
            &account,
            super::auth::field(&form, "memory"),
            super::auth::field(&form, "cpu"),
        )
        .await
        {
            return Ok(super::auth::back_with_error(here, &reason));
        }

        Ok(super::auth::see_other(&format!("{here}#reserves")))
    }

    #[post("/node/capabilities")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_capabilities(&self, request: Request) -> RestResult<Response> {
        let here = &NodeTab::Allows.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        // The one POST under `/node` that had no admin check, and the
        // omission was not small. `evict_ungranted` and
        // `release_ungranted` run at boot and act on exactly these
        // values, so turning `host` off here throws off every replica
        // this node holds for anybody, and turning `edge` off releases
        // every name it claims — from any account with a session.
        //
        // The page that draws the form is behind `is_admin` already, so
        // this was hidden and not enforced, which is the distinction
        // every island in this console is written around.
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let asked = crate::dtos::requests::set_capabilities::SetCapabilities {
            provides: super::auth::capabilities_ticked(&form, ""),
            owns: form.contains_key("own"),
        };
        if let Err(reason) = crate::usecases::node_user_set_capabilities::run(
            &self.state.database,
            &self.state.config,
            &self.state.deployer,
            &account,
            &asked,
        )
        .await
        {
            return Ok(super::auth::back_with_error(here, &reason));
        }

        Ok(super::auth::see_other(here))
    }

    /// Redeploy every managed engine here, because something they read
    /// at startup has changed.
    ///
    /// Queued rather than done: a deployment is this node's own job,
    /// which is what "obeying is local" has meant since phase 3, and a
    /// form that waited for three databases to restart is a form that
    /// times out.
    async fn redeploy_databases(&self) {
        let Ok(services) = crate::repositories::services::all(&self.state.database, None).await
        else {
            return;
        };
        for service in services.iter().filter(|service| service.kind.is_managed()) {
            let command = crate::deploy::jobs::DeployService {
                service_id: service.id.clone(),
                release_id: None,
            };
            if let Err(error) =
                wabot::async_jobs::run_command(&self.state.container, &command).await
            {
                tracing::error!(service = %service.slug, %error, "could not queue it");
            }
        }
    }

    /// What this node backs itself up to, and when.
    ///
    /// **The destination is checked here and not only when the run comes
    /// round.** A schedule saved against `sftp://…` — one letter from a
    /// scheme this understands — is a job that fails at three in the
    /// morning for a reason that was available the moment somebody typed
    /// it. Same rule as the certificate form, which reads a pair of files
    /// before it accepts them.
    #[post("/node/backups")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_backups(&self, request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let asked = crate::dtos::requests::set_backup_plan::SetBackupPlan {
            destination: super::auth::field(&form, "destination").to_string(),
            cadence: super::auth::field(&form, "cadence").to_string(),
            hour: super::auth::field(&form, "hour").to_string(),
            weekday: super::auth::field(&form, "weekday").to_string(),
            keep_days: super::auth::field(&form, "keep_days").to_string(),
            // Presence, not value: an unticked checkbox sends nothing at
            // all, so this is the last place that can tell it from a
            // form that never carried the field.
            archiving: form.contains_key("archiving"),
        };

        let saved = crate::usecases::node_user_set_backup_plan::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await;

        match saved {
            Ok(saved) => {
                // Something the managed engines read at startup moved, so
                // they have to come back with it.
                if saved.archiving_changed {
                    self.redeploy_databases().await;
                }
                Ok(super::auth::see_other(here))
            }
            Err(reason) => Ok(super::auth::back_with_error(here, &reason)),
        }
    }

    /// Store the bucket's credential — in the file, never in the
    /// database.
    ///
    /// Jorge asked for the mechanism: a bucket used to need an SSH
    /// session and a text editor. What did not move is *where* it goes,
    /// and the reasoning is on `config::BackupConfig`: a key that can
    /// read every backup in the network must not be inside the thing
    /// being backed up.
    ///
    /// **Asked of the provider before it is written**, when the
    /// destination is a bucket. The call is a listing, which is what a
    /// backup starts with anyway — so a credential that saves is one that
    /// works, and a typo is refused here rather than at three in the
    /// morning by a schedule nobody is watching.
    #[post("/node/backups/credentials")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_bucket_credentials(&self, request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = crate::dtos::requests::set_bucket_credentials::SetBucketCredentials {
            access_key_id: super::auth::field(&form, "access_key_id").to_string(),
            secret_access_key: super::auth::field(&form, "secret_access_key").to_string(),
            region: super::auth::field(&form, "region").to_string(),
            endpoint: super::auth::field(&form, "endpoint").to_string(),
        };
        match crate::usecases::node_user_set_bucket_credentials::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await
        {
            Ok(()) => Ok(super::auth::see_other(here)),
            Err(reason) => Ok(super::auth::back_with_error(here, &reason)),
        }
    }

    /// Take the credential out of the file.
    #[post("/node/backups/credentials/forget")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn forget_bucket_credentials(&self, _request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        if let Err(reason) = crate::usecases::node_user_forget_bucket_credentials::run(
            &self.state.database,
            &self.state.config,
            &account,
        )
        .await
        {
            return Ok(super::auth::back_with_error(here, &reason));
        }
        Ok(super::auth::see_other(here))
    }

    /// Take one now.
    ///
    /// Started and not awaited: a backup is minutes of `pg_basebackup`
    /// and file copying, and a browser waiting on that times out
    /// somewhere in the middle — leaving the operator with no page and a
    /// backup still running. The page it comes back to says one is
    /// running, and how the last one went.
    #[post("/node/backups/now")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn back_up_now(&self, _request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        // Refused rather than queued. Two of these is two copies of every
        // database at once on a machine with one core, and a second click
        // on a button that has not answered yet is the ordinary way to
        // ask for that.
        match crate::usecases::node_user_back_up_now::run(
            &self.state.database,
            &self.state.config,
            &account,
        )
        .await
        {
            Ok(()) => Ok(super::auth::see_other(here)),
            Err(reason) => Ok(super::auth::back_with_error(here, &reason)),
        }
    }

    /// Mint a join token for a node that does not exist here yet.
    ///
    /// Everything the other machine needs is decided here, because
    /// everything it needs is this node's to decide: the address on
    /// this node's overlay, the key it will talk to, and the secret it
    /// will authenticate with. The token is shown once and then only
    /// its hash is kept.
    #[post("/network/enrol")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn enrol(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        // Enrolling a node is not a project-level decision: it grants
        // whoever holds the token a place on this node's network.
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let here = "/network/invite";

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        // Optional, and it is **not** the node's name: a node's name is
        // its own, arrives on the join callback, and is what the list of
        // nodes shows. This is a note on a token that has not been spent
        // yet, for the operator who minted three of them this afternoon
        // — so it is bounded, and it is not required.
        let asked = crate::dtos::requests::mint_join_token::MintJoinToken {
            name: super::auth::field(&form, "name").to_string(),
            requires: super::auth::capabilities_ticked(&form, "require-"),
            offers: super::auth::capabilities_ticked(&form, "offer-"),
        };
        let token = match crate::usecases::node_user_mint_join_token::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await
        {
            Ok(token) => token,
            Err(reason) => return Ok(super::auth::back_with_error(here, &reason)),
        };

        // Through the reveal store, not the query string: the token is
        // the secret, and a query string is read by the address bar, the
        // history and every refresh.
        let nonce = self
            .state
            .reveals
            .stash(format!("wabot-deploy join {token}"));
        Ok(super::auth::see_other(&format!(
            "/network/invite/shown?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("nonce", &nonce)
                .finish()
        )))
    }

    /// Take instructions from the node that minted this token.
    ///
    /// The console's door onto the same work `wabot-deploy join` does,
    /// and the reason it exists: a node that is already installed and
    /// already answering is one somebody is looking at through a
    /// browser, and telling them to go and find its terminal for a
    /// one-field form is telling them to go somewhere else to do the
    /// thing they are already here for.
    ///
    /// Inline rather than queued, unlike a deployment. It is one
    /// request with a timeout, and its whole value is the answer: a
    /// join that failed has a reason, and a reason nobody sees is the
    /// failure this console exists to prevent.
    #[post("/network/join")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn join(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        // Granting another node authority over this one is the largest
        // thing anybody can do from this console.
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let here = "/network/join";

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        use crate::dtos::responses::join_token_spent::Spent;
        let asked = crate::dtos::requests::spend_join_token::SpendJoinToken {
            token: super::auth::field(&form, "token").to_string(),
            agreed: form.contains_key("agreed"),
            // The form's spelling stops here. A checkbox arrives only
            // when ticked, so presence is the answer.
            accepted: crate::network::capability::Capability::ALL
                .into_iter()
                .filter(|capability| form.contains_key(&format!("accept-{}", capability.name())))
                .collect(),
        };
        match crate::usecases::node_user_spend_join_token::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await
        {
            Ok(Spent::Review(token)) => {
                let nonce = self.state.reveals.stash(token);
                Ok(super::auth::see_other(&format!(
                    "{here}?{}",
                    form_urlencoded::Serializer::new(String::new())
                        .append_pair("review", &nonce)
                        .finish()
                )))
            }
            Ok(Spent::Joined(authority)) => Ok(super::auth::see_other(&format!(
                "{here}?{}",
                form_urlencoded::Serializer::new(String::new())
                    .append_pair("joined", &authority)
                    .finish()
            ))),
            Err(reason) => Ok(super::auth::back_with_error(here, &reason)),
        }
    }

    /// Withdraw a token nobody has spent.
    #[post("/network/enrolments/:enrolment/withdraw")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn withdraw(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = super::auth::segments(&path).get(2) {
            if let Err(reason) = crate::usecases::node_user_withdraw_enrolment::run(
                &self.state.database,
                &account,
                id,
            )
            .await
            {
                return Ok(super::auth::back_with_error("/network/invite", &reason));
            }
        }
        Ok(super::auth::see_other("/network/invite"))
    }

    /// Take a node out of service, or put it back.
    ///
    /// **The mark and the moving are one press**, because a mark without
    /// the moving is a node quietly holding work nobody will ever choose
    /// it for again, and moving without the mark is a drain the planner
    /// undoes on its next decision.
    ///
    /// What comes back is a count of rows moved *here* and a sentence
    /// per copy that could not be — never "drained", because the far
    /// node stops when it next collects and this console cannot know
    /// that it did.
    #[post("/network/:node/drain")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn drain_node(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(super::auth::see_other("/network"));
        };
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let here = format!("/network/{id}");

        use crate::dtos::responses::node_drained::Drained;
        match crate::usecases::node_user_drain_node::run(
            &self.state.database,
            &self.state.deployer,
            &account,
            id,
            super::auth::field(&form, "draining") == "on",
        )
        .await
        {
            Ok(Drained::BackInService) => Ok(super::auth::see_other(&here)),
            Ok(Drained::Moved { moved, refused }) if !refused.is_empty() => {
                Ok(super::auth::back_with_error(
                    &here,
                    &crate::views::texts::sentences::copies_moved(moved, &refused),
                ))
            }
            Ok(Drained::Moved { .. }) => Ok(super::auth::see_other(&here)),
            Err(reason) => Ok(super::auth::back_with_error(&here, &reason)),
        }
    }

    #[post("/network/forget/:node")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn forget_node(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = super::auth::segments(&path).get(2) {
            if let Err(reason) =
                crate::usecases::node_user_forget_node::run(&self.state.database, &account, id)
                    .await
            {
                return Ok(super::auth::back_with_error("/network", &reason));
            }
        }
        Ok(super::auth::see_other("/network"))
    }

    /// Stop taking instructions from a node.
    ///
    /// The half of joining that makes it not a one-way door. It takes
    /// effect here and nowhere else — the other node keeps whatever it
    /// recorded, and finds out by being refused.
    #[post("/network/revoke/:node")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revoke_authority(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = super::auth::segments(&path).get(2) {
            if let Err(reason) =
                crate::usecases::node_user_revoke_authority::run(&self.state.database, &account, id)
                    .await
            {
                return Ok(super::auth::back_with_error("/network", &reason));
            }
        }
        Ok(super::auth::see_other("/network"))
    }

    /// What this node agrees to do for one other node.
    ///
    /// The whole set each time, ticked and unticked together: a form that
    /// could only add would make "I have changed my mind about that one"
    /// impossible to say. Withdrawing needs a row of its own —
    /// `capability::set_granted` writes it — because the boot pass that
    /// re-reads what an enrolment offered would otherwise put it back.
    #[post("/network/:node/grants")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_grants(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).map(|id| id.to_string()) else {
            return Ok(super::auth::see_other("/network"));
        };
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let here = format!("/network/{id}");
        let asked = crate::dtos::requests::set_grants::SetGrants {
            node: id.to_string(),
            agreed: super::auth::capabilities_ticked(&form, ""),
        };
        match crate::usecases::node_user_set_grants::run(&self.state.database, &account, &asked)
            .await
        {
            Ok(()) => Ok(super::auth::see_other(&here)),
            Err(reason) => Ok(super::auth::back_with_error(&here, &reason)),
        }
    }

    /// Ask a node to be running a release.
    ///
    /// Two writes and neither is the instruction: the decision goes on
    /// that node's row, and `dispatch_versions` turns it into an errand —
    /// here and at every boot. A form that queued directly would be an
    /// instruction that exists only if the button was pressed while both
    /// machines were up.
    #[post("/network/:node/version")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_wanted_version(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).map(|id| id.to_string()) else {
            return Ok(super::auth::see_other("/network"));
        };
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let here = format!("/network/{id}");
        match crate::usecases::node_user_set_wanted_version::run(
            &self.state.database,
            &account,
            &id,
            super::auth::field(&form, "version"),
        )
        .await
        {
            Ok(()) => Ok(super::auth::see_other(&here)),
            Err(reason) => Ok(super::auth::back_with_error(&here, &reason)),
        }
    }

    /// Ask a node to do something once.
    ///
    /// No deduplication and no desired state: pressing it twice asks
    /// twice, and each asking is its own errand with its own id. What that
    /// id buys is that the *same* asking cannot be obeyed twice, which
    /// matters because the thing it asks for kills the process that would
    /// have reported it done.
    #[post("/network/:node/command")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn ask_node_once(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).map(|id| id.to_string()) else {
            return Ok(super::auth::see_other("/network"));
        };
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let here = format!("/network/{id}");
        let action = match super::auth::field(&form, "action") {
            "restart" => network::errand::Action::Restart,
            "back-up" => network::errand::Action::BackUp,
            // Not `Action::Unknown`: that is what a *newer node's* word
            // parses to on the way in, and sending it would be this
            // console asking for something it has no name for.
            other => {
                tracing::warn!(action = %other, "no such thing to ask a node for");
                return Ok(super::auth::back_with_error(&here, "no such action"));
            }
        };
        match crate::usecases::node_user_ask_node_once::run(
            &self.state.database,
            &account,
            &id,
            action,
        )
        .await
        {
            Ok(()) => Ok(super::auth::see_other(&here)),
            Err(reason) => Ok(super::auth::back_with_error(&here, &reason)),
        }
    }

    /// Choose where this name's certificate comes from.
    ///
    /// The file pair is read and checked *here*, before the choice is
    /// stored. Storing a policy that cannot work would make the
    /// renewal loop fail on every pass with the reason in the journal,
    /// and this console exists so that reasons do not live there. The
    /// worse case is the one this refuses outright: a certificate and
    /// key that do not match install cleanly and then break every
    /// handshake, including the one serving this page.
    #[post("/node/certificate/source")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_certificate_source(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let here = &self_path(&self.state.database).await;

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        // The node's own name, or the fallback while it has none — the
        // same key the renewal loop will look this up under.
        let name = match crate::usecases::node_user_set_certificate_source::run(
            &self.state.database,
            &self.state.config,
            &account,
        )
        .await
        {
            Ok(name) => name,
            Err(reason) => return Ok(super::auth::back_with_error(here, &reason)),
        };

        let renew_with = match source_from(&form, &name) {
            Ok(renew_with) => renew_with,
            Err(reason) => return Ok(super::auth::back_with_error(here, &reason)),
        };

        if let Err(error) =
            store_source(&self.state.database, &self.state.config, &name, &renew_with).await
        {
            return Ok(super::auth::back_with_error(here, &error));
        }
        // The loop installs it. Same reason as saving a domain: this
        // console asks, it does not issue.
        self.state.certificates.now();

        Ok(super::auth::see_other(here))
    }

    /// The node's live state, as server-sent events.
    ///
    /// One stream for both halves of this page rather than two. Memory
    /// wants a tick; a certificate wants to be told. Merging them keeps
    /// the console to a single `EventSource` — see the module docs —
    /// and means the certificate answer lands the moment the loop has
    /// it instead of up to two seconds later.
    ///
    /// `#[raw]`, because the body is a stream that never ends and the
    /// JSON path buffers a value. The stream stops when the browser
    /// disconnects — the send fails, and the task ends with it.
    #[get("/network/:node/live")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn live_stream(&self, request: Request) -> RestResult<Response> {
        if !signed_in(&self.auth)
            .is_some_and(|account| crate::authz::node_user::manages_the_machine(&account))
        {
            // A stream is not a page, so this is a status rather than a
            // redirect: an EventSource cannot follow one usefully.
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }

        let path = request.uri().path().to_string();
        let id = super::auth::segments(&path)
            .get(1)
            .map(|id| id.to_string())
            .unwrap_or_default();
        // Only this node's own page has a stream. What it carries — a
        // memory reading and a certificate — are answers only the
        // machine itself can give, so a stream for a joined node would
        // be this one reporting its own figures under somebody else's
        // name.
        let mine = network::me(&self.state.database)
            .await
            .unwrap_or_default()
            .is_some_and(|me| me.id == id);
        // A joined node's page has a stream too now, and carries only
        // what this node can honestly say about it: the errands it was
        // sent, and how far each has got. Memory and certificates stay
        // behind `mine` — those are answers only that machine can give,
        // and streaming this one's figures under its name would be the
        // page inventing them.
        if !mine
            && network::find(&self.state.database, &id)
                .await
                .unwrap_or_default()
                .is_none()
        {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }

        let state = self.state.clone();
        let mut phase = state.certificates.watch();
        let stream = async_stream::stream! {
            // The previous CPU reading, which is the whole of why this
            // figure can exist here and nowhere else: a percentage is the
            // difference between two samples, and a stream is the one
            // place that naturally has both. The first tick carries none —
            // there is nothing to subtract from — and the page shows a
            // dash rather than a number that would mean "since boot".
            let mut before: Option<crate::node::cpu::Sample> = None;
            loop {
                let snapshot = state.deployer.memory().await;
                let now = crate::node::cpu::sample(&state.deployer.cgroups().await);
                let busy = before
                    .as_ref()
                    .and_then(|before| crate::node::cpu::between(before, &now));
                before = Some(now);
                let facts = super::certificate_facts(&state).await;
                let domain =
                    crate::node::settings::domain(&state.database, &state.config).await;
                let certificate = CertificateState::read(
                    &facts,
                    domain.as_deref(),
                    crate::edge::policy::for_name(
                        &state.database,
                        &state.config,
                        domain.as_deref().unwrap_or(crate::edge::certs::FALLBACK_NAME),
                    )
                    .await
                    .last_error
                    .or(crate::node::settings::acme_error(&state.database).await),
                    state.certificates.phase(),
                    state.config.acme.disabled,
                );
                let payload = serde_json::to_string(&Live {
                    inner: cells(&snapshot, busy.as_ref()),
                    certificate: crate::views::components::node_certificate::certificate_cells(&certificate, &facts, domain.as_deref()),
                    errands: errand_cells(&state.database, &id).await,
                })
                .unwrap_or_else(|_| "{}".into());
                yield Ok::<_, std::convert::Infallible>(
                    wabot::rest::axum::body::Bytes::from(format!("data: {payload}\n\n")),
                );

                // Whichever comes first. The tick is for memory, which
                // nothing announces; the watch is for the certificate,
                // which does.
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                    changed = phase.changed() => {
                        // The sender lives in the shared state, so this
                        // only ends if the whole node is going down.
                        if changed.is_err() { break; }
                    }
                }
            }
        };

        Ok(Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            // Every hop in between has to be told, or a proxy holds the
            // stream until it has "enough" and the page never updates.
            .header(header::CACHE_CONTROL, "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .expect("a constant response is well-formed"))
    }
}

/// The whole payload: the memory reading, and the certificate beside
/// it. Flattened so the script reads `data.cells` exactly as it did
/// when memory was all this stream carried.
#[derive(serde::Serialize)]
struct Live {
    #[serde(flatten)]
    inner: Cells,
    certificate: CertificateCells,
    /// One per errand this node sent that one, keyed by id.
    ///
    /// Empty on this node's own page — it sends itself none — and the
    /// only thing on a joined node's, where the memory and certificate
    /// figures would be this machine's own under somebody else's name.
    errands: BTreeMap<String, ErrandCell>,
}

/// One errand row, as its badge reads.
#[derive(serde::Serialize)]
struct ErrandCell {
    badge: &'static str,
    dot: &'static str,
    word: &'static str,
    /// The reason, when it was refused. Empty otherwise.
    failure: String,
}

/// How far each errand sent to `node_id` has got.
///
/// A node collects on a fifteen-second timer and reports when it is
/// done, so "waiting to be collected" is precisely the state somebody
/// stares at after pressing the button — and it used to stay on screen
/// long after the far node had finished.
async fn errand_cells(
    database: &wabot::sqlite::SqliteDatabase,
    node_id: &str,
) -> BTreeMap<String, ErrandCell> {
    let mut cells = BTreeMap::new();
    let Ok(orders) = network::errand::all(database).await else {
        return cells;
    };

    for order in orders
        .into_iter()
        .filter(|record| record.node_id == node_id)
    {
        let cell = if let Some(reason) = &order.error {
            ErrandCell {
                badge: "badge badge-danger",
                dot: "dot dot-danger",
                word: "Refused",
                failure: reason.clone(),
            }
        } else if order.done() {
            ErrandCell {
                badge: "badge badge-success",
                dot: "dot dot-success",
                word: "Done",
                failure: String::new(),
            }
        } else if order.taken_at.is_some() {
            ErrandCell {
                badge: "badge badge-info",
                dot: "dot dot-info dot-pulse",
                word: "Collected",
                failure: String::new(),
            }
        } else {
            ErrandCell {
                badge: "badge badge-info",
                dot: "dot dot-info",
                word: "Waiting to be collected",
                failure: String::new(),
            }
        };
        cells.insert(order.id.clone(), cell);
    }
    cells
}

/// What the script writes where.
///
/// Built here rather than in the browser so the two halves cannot
/// disagree about formatting: the same `human()` renders the first
/// paint and every update after it.
#[derive(serde::Serialize)]
pub(crate) struct Cells {
    pub cells: BTreeMap<String, String>,
    pub bars: BTreeMap<String, String>,
}

fn cells(snapshot: &Snapshot, busy: Option<&crate::node::cpu::Busy>) -> Cells {
    let containers = snapshot.containers_total();
    let runtime = snapshot.containerd + snapshot.shims;

    let cells = BTreeMap::from([
        (
            "summary".to_string(),
            format!(
                "{} of {}",
                memory::human(snapshot.used()),
                memory::human(snapshot.total)
            ),
        ),
        ("node".to_string(), memory::human(snapshot.node)),
        ("containerd".to_string(), memory::human(snapshot.containerd)),
        ("shims".to_string(), memory::human(snapshot.shims)),
        ("containers".to_string(), memory::human(containers)),
        ("rest".to_string(), memory::human(snapshot.rest())),
        (
            "swap".to_string(),
            format!(
                "{} of {}",
                memory::human(snapshot.swap_used),
                memory::human(snapshot.swap_total)
            ),
        ),
        // A dash until there are two readings to subtract. Not "0 %": a
        // machine at nought is a machine somebody would believe.
        // Millicores, against what the machine has: a thousand is one
        // core, and the capacity is what makes the first number mean
        // something without knowing the box.
        (
            "cpu".to_string(),
            busy.map(|busy| format!("{}m of {}m", busy.node, busy.capacity))
                .unwrap_or_else(|| "—".into()),
        ),
    ]);

    let bars = BTreeMap::from([
        (
            "node".to_string(),
            crate::views::components::node_usage::percent(snapshot, snapshot.node),
        ),
        (
            "runtime".to_string(),
            crate::views::components::node_usage::percent(snapshot, runtime),
        ),
        (
            "containers".to_string(),
            crate::views::components::node_usage::percent(snapshot, containers),
        ),
        (
            "rest".to_string(),
            crate::views::components::node_usage::percent(snapshot, snapshot.rest()),
        ),
    ]);

    Cells { cells, bars }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::Config;
    use crate::console::tests::Console;
    use crate::network::Kind;
    use wabot::rest::axum::http::StatusCode;

    fn snapshot() -> Snapshot {
        Snapshot {
            total: 1024 * 1024 * 1024,
            available: 512 * 1024 * 1024,
            node: 12 * 1024 * 1024,
            containerd: 30 * 1024 * 1024,
            shims: 11 * 1024 * 1024,
            shim_count: 1,
            containers: BTreeMap::from([("demo.web".into(), 13 * 1024 * 1024)]),
            ..Default::default()
        }
    }

    /// The contract between the two halves: every cell the script
    /// writes has to exist in the page it writes into, or the first
    /// update silently does nothing.
    #[test]
    fn every_streamed_cell_has_a_place_on_the_page() {
        let snapshot = snapshot();
        let rendered = crate::views::components::node_usage::memory_card(
            &snapshot,
            &Default::default(),
            (None, None),
        )
        .render()
        .into_inner();
        let payload = cells(&snapshot, None);

        for key in payload.cells.keys() {
            // Swap is only rendered when the machine has some.
            if key == "swap" {
                continue;
            }
            assert!(
                rendered.contains(&format!(r#"data-cell="{key}""#)),
                "nothing on the page shows {key}:\n{rendered}"
            );
        }
        for key in payload.bars.keys() {
            assert!(
                rendered.contains(&format!(r#"data-bar="{key}""#)),
                "no bar for {key}"
            );
        }
    }

    /// A bar the browser will actually take.
    ///
    /// The stream assigns these to `style.width`, which refuses anything
    /// it cannot parse and refuses it silently — so `width:12.34%`, right
    /// as an attribute, was a meter that never moved after the first
    /// paint while the numbers beside it changed every second. The
    /// attribute and the assignment want different strings, and only one
    /// of them says so out loud.
    #[test]
    fn a_streamed_bar_is_a_value_and_the_attribute_is_a_declaration() {
        let snapshot = snapshot();

        for (key, value) in &cells(&snapshot, None).bars {
            assert!(
                !value.contains("width"),
                "{key} carries a declaration where a value belongs: {value}"
            );
            assert!(value.ends_with('%'), "{key} is not a percentage: {value}");
        }

        // And the first paint still needs the whole declaration, because
        // there it is the attribute.
        assert!(crate::views::components::node_usage::memory_card(
            &snapshot,
            &Default::default(),
            (None, None)
        )
        .render()
        .into_inner()
        .contains("style=\"width:"));
    }

    /// Every meter's width goes through `bar_width`.
    ///
    /// The test above asserts the shape for the *memory* card, and the
    /// disk card shipped with its own closure formatting a bare
    /// `7.0000%` — beside a correct helper of the same name in the same
    /// file. A `style` attribute wants a declaration, so the browser
    /// discarded it and the bar rendered flat while the figures under it
    /// were right: it read as a node using no disk rather than as a broken
    /// bar. Reported by Jorge.
    ///
    /// So this reads the source instead of one card's output. One place to
    /// be right is worth nothing unless everything goes through it, and a
    /// third card is exactly how this happened a second time.
    #[test]
    fn every_meter_width_is_a_declaration() {
        let source = include_str!("nodes.rs");
        // Assembled rather than written out, or this scan finds its own
        // predicate and reports the line it is reading from.
        let needle = concat!("style", "=(");
        let mut wrong = Vec::new();
        for (number, line) in source.lines().enumerate() {
            if line.trim_start().starts_with("//") || !line.contains(needle) {
                continue;
            }
            if !line.contains("crate::views::components::node_usage::width(") {
                wrong.push(number + 1);
            }
        }
        assert!(
            wrong.is_empty(),
            "these write a style attribute without going through bar_width: {wrong:?}"
        );
    }

    /// And what it produces is a declaration, in both directions of the
    /// confusion that caused this twice.
    #[test]
    fn a_meter_width_is_a_declaration_and_not_a_bare_value() {
        assert_eq!(
            crate::views::components::node_usage::bar_width(7.5),
            "width:7.5000%"
        );
        // Zero is still sized, rather than left for whatever was there.
        assert_eq!(
            crate::views::components::node_usage::bar_width(0.0),
            "width:0.0000%"
        );
    }

    /// The figures are formatted once, on the server, so the first
    /// paint and every update after it read the same.
    #[test]
    fn the_first_paint_and_the_stream_agree() {
        let snapshot = snapshot();
        let rendered = crate::views::components::node_usage::memory_card(
            &snapshot,
            &Default::default(),
            (None, None),
        )
        .render()
        .into_inner();

        for value in cells(&snapshot, None).cells.values() {
            if value.contains("of 0 B") {
                continue; // swap, which this machine does not have
            }
            assert!(
                rendered.contains(value.as_str()),
                "the page does not show {value}"
            );
        }
    }

    /// Empty means the default, and a number survives the round trip.
    #[tokio::test]
    async fn a_reserve_is_stored_and_cleared_from_the_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($memory:expr) => {{
                let response = console
                    .harness
                    .post("/node/reserves")
                    .header("cookie", &cookie)
                    .form(&[("memory", $memory), ("cpu", "")])
                    .send()
                    .await;
                response.header("location").unwrap_or_default().to_string()
            }};
        }

        let saved = save!("512 MB");
        // Back to the tab the form is on. It was `/node`, which is a
        // redirect to the first tab — so saving a reserve landed on the
        // domain page and the fragment pointed at a card that was not
        // there.
        assert!(saved.ends_with("/node/resources#reserves"), "{saved}");
        assert_eq!(
            crate::node::settings::memory_reserve(&console.database).await,
            Some(512 * 1024 * 1024)
        );

        // Empty is "whatever this machine's size implies", not nought.
        save!("");
        assert_eq!(
            crate::node::settings::memory_reserve(&console.database).await,
            None,
            "empty is the default, not zero"
        );

        // And a size the parser refuses never reaches the row.
        let refused = save!("lots");
        assert!(refused.contains("error="), "{refused}");
    }

    /// Every capability has a checkbox, because the handler writes a
    /// setting for every one of them.
    ///
    /// **The form used to draw two of three.** So saving it turned
    /// `store` off in silence: the node stopped being offered as
    /// somewhere a standby could go, and refused the errands for the ones
    /// it already held. The card is generated from `Capability::ALL` now
    /// and the words come out of an exhaustive `match`, which makes the
    /// mistake a compile error — this is the belt.
    #[tokio::test]
    async fn every_capability_has_a_checkbox_on_the_form() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let body = console
            .harness
            .get(&NodeTab::Allows.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        for capability in network::capability::Capability::ALL {
            assert!(
                body.contains(&format!("name=\"{}\"", capability.name())),
                "{} has no checkbox, and saving this form would turn it off: {body}",
                capability.name()
            );
        }
        assert!(
            body.contains("name=\"own\""),
            "and the switch that is not a capability: {body}"
        );
    }

    /// What this node agrees to do for the whole network is not a
    /// member's decision.
    ///
    /// **This guard was missing**, and it was the only POST under `/node`
    /// without one — every other form in this controller asks. What it
    /// cost is not a setting written wrongly: `evict_ungranted` and
    /// `release_ungranted` run at boot and act on these values, so a
    /// member turning `host` off threw off every replica this node holds
    /// for anybody, and turning `edge` off released every name it claims.
    /// The form was hidden behind `is_admin` and the handler was not,
    /// which is the difference this console is written around, and the
    /// tests around it all signed in as the administrator.
    #[tokio::test]
    async fn only_an_admin_may_change_what_this_node_provides() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        let response = console
            .harness
            .post("/node/capabilities")
            .header("cookie", &member)
            // Every box unticked, which is how this form says "off".
            .form::<&str, &str>(&[])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));

        for capability in network::capability::Capability::ALL {
            assert!(
                network::capability::provides(&console.database, capability).await,
                "a member turned {} off for the whole node",
                capability.name()
            );
        }
        assert!(
            crate::network::capability::owns(&console.database).await,
            "and stopped it owning anything"
        );
    }

    /// A node cannot stop being an owner while it still owns something.
    ///
    /// Its projects would be in a state nothing can act on: not editable,
    /// because the node no longer originates work; not evictable, because
    /// they are its own and no other node placed them. Like a foreign
    /// key, and the message names the count so it is a thing somebody can
    /// go and do.
    #[tokio::test]
    async fn a_node_that_owns_projects_cannot_stop_being_an_owner() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            () => {
                console
                    .harness
                    .post("/node/capabilities")
                    .header("cookie", &cookie)
                    .form(&[("host", "1"), ("edge", "1")])
                    .send()
                    .await
                    .header("location")
                    .unwrap_or_default()
                    .to_string()
            };
        }

        crate::repositories::projects::create(&console.database, "mine")
            .await
            .expect("a project of its own");

        let refused = save!();
        assert!(refused.contains("error="), "{refused}");
        assert!(
            crate::network::capability::owns(&console.database).await,
            "and nothing was written"
        );

        // With nothing of its own left, the same click is accepted.
        let projects = crate::repositories::projects::all(&console.database)
            .await
            .expect("projects");
        for project in projects {
            crate::repositories::projects::delete(&console.database, &project.id)
                .await
                .expect("removed");
        }
        let saved = save!();
        assert!(!saved.contains("error="), "{saved}");
        assert!(!crate::network::capability::owns(&console.database).await);
    }

    /// This node's settings are tabs, like a service's.
    ///
    /// Three unrelated cards in one column, and Jorge asked for the same
    /// shape he had asked for on a service. One URL each, so a tab can be
    /// linked and read with scripting off, and the page loads only what it
    /// shows — the domain tab is the only one that reads a certificate off
    /// disk.
    #[tokio::test]
    async fn the_nodes_settings_are_tabs() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! get {
            ($path:expr) => {
                console
                    .harness
                    .get($path)
                    .header("cookie", &cookie)
                    .send()
                    .await
            };
        }

        // The bare path names the first tab, and so does a word that is
        // not one — a hand-typed URL, or a link from before there were
        // tabs at all.
        assert_eq!(get!("/node").header("location"), Some("/node/domain"));
        assert_eq!(
            get!("/node/nonsense").header("location"),
            Some("/node/domain")
        );

        // Each card on exactly one tab. A card on two is a card somebody
        // saves twice.
        let mut seen = Vec::new();
        for tab in NodeTab::all() {
            let body = get!(&tab.path()).body;
            assert!(
                body.contains(r#"aria-current="page""#),
                "the {} tab does not say it is current: {body}",
                tab.slug()
            );
            for (form, name) in [
                ("/node/certificate", "certificate"),
                ("/node/capabilities", "capabilities"),
                ("/node/reserves", "reserves"),
                ("/node/backups", "backups"),
            ] {
                if body.contains(form) {
                    seen.push((name, tab.slug()));
                }
            }
        }
        assert_eq!(
            seen,
            vec![
                ("certificate", "domain"),
                ("capabilities", "allows"),
                ("reserves", "resources"),
                // The tab is `backup` and what it submits to is
                // `backups`: a GET view and a POST can share a path, but
                // a form whose action is the page it sits on reads as a
                // coincidence rather than a decision.
                ("backups", "backup"),
            ],
            "a form is missing, or on more than one tab"
        );
    }

    /// The tab Jorge asked for, and what had to move onto it.
    ///
    /// The write-ahead log switch was on Capabilities, as a third thing
    /// the node either does or does not do. It belongs beside the
    /// schedule: point-in-time recovery is how far a restore *reaches*
    /// from a backup, and with no backup to anchor it the log is disk
    /// nothing can use.
    #[tokio::test]
    async fn the_backup_tab_carries_the_plan_and_the_log_switch() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(body.contains(r#"action="/node/backups""#), "{body}");
        assert!(
            body.contains(r#"name="archiving""#),
            "the log switch is here now: {body}"
        );
        assert!(
            body.contains(r#"action="/node/backups/now""#),
            "and the button that takes one now: {body}"
        );
        // The island hides what does not apply; it does not build the
        // form. Every field is in the HTML the node produced, which is
        // what makes this page work with scripting off.
        assert!(body.contains(r#"data-island="fields""#), "{body}");
        assert!(body.contains(r#"data-when="cadence=weekly""#), "{body}");
        conditions_name_real_controls(&body);

        // And it is gone from where it was. A switch on two tabs is a
        // switch somebody saves twice, with two different answers.
        let allows = console
            .harness
            .get(&NodeTab::Allows.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            !allows.contains(r#"name="archiving""#),
            "the log switch is still on Capabilities: {allows}"
        );
    }

    /// A plan survives the form and comes back selected.
    ///
    /// The second half is the half that has been wrong here before:
    /// `selected` is read by *presence*, so a value rendered as
    /// `selected=(false)` selects — which is how a selector came to name
    /// the wrong project. Every option on this form branches.
    #[tokio::test]
    async fn a_backup_plan_is_stored_and_comes_back_selected() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let saved = console
            .harness
            .post("/node/backups")
            .header("cookie", &cookie)
            .form(&[
                ("destination", "/mnt/backups"),
                ("cadence", "weekly"),
                ("hour", "4"),
                ("weekday", "2"),
                ("keep_days", "30"),
            ])
            .send()
            .await;
        assert_eq!(saved.header("location"), Some("/node/backup"));

        let plan = crate::node::backups::plan(&console.database).await;
        assert_eq!(plan.destination.as_deref(), Some("/mnt/backups"));
        assert_eq!(plan.cadence, crate::node::backups::Cadence::Weekly);
        assert_eq!(plan.hour, 4);
        assert_eq!(plan.weekday, 2);
        assert_eq!(plan.keep_days, 30);

        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains(r#"value="weekly" selected"#), "{body}");
        assert!(body.contains(r#"value="4" selected"#), "the hour: {body}");
        assert!(body.contains(r#"value="2" selected"#), "the day: {body}");
        assert!(body.contains(r#"value="30""#), "the window: {body}");
    }

    /// A destination that cannot work is refused when it is typed.
    ///
    /// Not when the run comes round. A schedule saved against
    /// `sftp://…` — one letter from a scheme this understands — is a job
    /// that fails at three in the morning for a reason that was
    /// available the moment somebody typed it.
    #[tokio::test]
    async fn a_destination_this_node_cannot_write_to_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($destination:expr) => {{
                let response = console
                    .harness
                    .post("/node/backups")
                    .header("cookie", &cookie)
                    .form(&[
                        ("destination", $destination),
                        ("cadence", "daily"),
                        ("hour", "3"),
                        ("weekday", "6"),
                        ("keep_days", "7"),
                    ])
                    .send()
                    .await;
                response.header("location").unwrap_or_default().to_string()
            }};
        }

        let refused = save!("sftp://host/backups");
        assert!(refused.contains("error="), "{refused}");
        // A bucket with no credentials on this node: the same answer, and
        // it names the file to put them in rather than the failure.
        let no_credentials = save!("s3://somebodys-bucket/nodes");
        assert!(no_credentials.contains("error="), "{no_credentials}");
        assert!(
            no_credentials.contains("card+below"),
            "and says where they go — the card on this tab, not an SSH session: {no_credentials}"
        );
        assert_eq!(
            crate::node::backups::plan(&console.database).await,
            crate::node::backups::Plan::default(),
            "a refused form must store nothing"
        );

        // And a window of no days, which would delete each backup as
        // soon as it was taken.
        let response = console
            .harness
            .post("/node/backups")
            .header("cookie", &cookie)
            .form(&[
                ("destination", ""),
                ("cadence", "daily"),
                ("hour", "3"),
                ("weekday", "6"),
                ("keep_days", "0"),
            ])
            .send()
            .await;
        let refused = response.header("location").unwrap_or_default().to_string();
        assert!(refused.contains("error="), "{refused}");
    }

    /// The switch reaches the row, and the databases are told.
    ///
    /// The telling is what makes it more than a checkbox: `archive_mode`
    /// is a postmaster setting written into the container's arguments, so
    /// without a redeployment the switch reads on, the page says on, and
    /// nothing archives.
    #[tokio::test]
    async fn the_log_switch_is_saved_from_the_backup_form() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($fields:expr) => {
                console
                    .harness
                    .post("/node/backups")
                    .header("cookie", &cookie)
                    .form($fields)
                    .send()
                    .await
            };
        }

        // A checkbox arrives only when ticked, so its absence is the
        // answer rather than a missing field.
        save!(&[("cadence", ""), ("keep_days", "7")]);
        assert!(!crate::node::settings::archiving(&console.database).await);

        save!(&[("cadence", ""), ("keep_days", "7"), ("archiving", "1")]);
        assert!(crate::node::settings::archiving(&console.database).await);
    }

    /// What is on the disk, and whose it is.
    ///
    /// A shared backup root holds several nodes' copies — that is what
    /// makes one bucket the answer for a network rather than for a
    /// machine — so a row about another node has to say so rather than
    /// read as this one's.
    #[tokio::test]
    async fn a_backup_on_the_disk_says_whose_it_is() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let write = |node: &str, at: i64, volumes: usize| {
            let into = console
                .data_dir
                .join("backups")
                .join("nodes")
                .join(node)
                .join(at.to_string());
            std::fs::create_dir_all(&into).expect("a backup directory");
            let manifest = serde_json::json!({
                "format": 1,
                "taken_by": "wabot-deploy 0.0.0-test",
                "taken_at": at,
                "node_id": node,
                "node_name": format!("{node}.example"),
                "volumes": (0..volumes).map(|index| serde_json::json!({
                    "container": format!("demo.orders-{index}"),
                    "bytes": 1024,
                    "how": "Consistent",
                })).collect::<Vec<_>>(),
                "images": [],
            });
            std::fs::write(
                into.join("manifest.json"),
                serde_json::to_vec(&manifest).expect("a manifest"),
            )
            .expect("write it");
        };

        let me = crate::network::me(&console.database)
            .await
            .expect("read")
            .expect("this node");
        write(&me.id, 1_700_000_000_000, 2);
        write("some-other-node", 1_600_000_000_000, 1);

        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("2 volume(s)"), "{body}");
        assert!(body.contains("this node"), "{body}");
        assert!(
            body.contains("some-other-node.example"),
            "the other node's row names it: {body}"
        );
    }

    /// A bucket is configured from the console, and the secret is not in
    /// the database.
    ///
    /// Jorge asked for the mechanism — configuring one was an SSH session
    /// and a text editor — and the answer is *which file*, not who writes
    /// it: a key that can read every backup in the network must not be
    /// inside the thing being backed up.
    #[tokio::test]
    async fn a_bucket_credential_is_stored_in_the_file_and_not_the_database() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let path = console.data_dir.join("config.toml");

        let saved = console
            .harness
            .post("/node/backups/credentials")
            .header("cookie", &cookie)
            .form(&[
                ("access_key_id", "AKIAEXAMPLE"),
                ("secret_access_key", "s3cr3t-from-the-provider"),
                ("region", "us-east-1"),
                ("endpoint", "https://s3.us-west-004.backblazeb2.com"),
            ])
            .send()
            .await;
        assert_eq!(saved.header("location"), Some("/node/backup"));

        let file = std::fs::read_to_string(&path).expect("the config file");
        assert!(file.contains("AKIAEXAMPLE"), "{file}");
        assert!(file.contains("s3cr3t-from-the-provider"), "{file}");

        // And nowhere near the database. Read as bytes because the
        // question is "is that string anywhere in this file", not "is
        // there a row" — a secret that leaked into any column at all
        // would travel in every backup.
        let db = std::fs::read(console.data_dir.join("db").join("node.db")).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&db).contains("s3cr3t-from-the-provider"),
            "the secret must not be in the node's own database"
        );

        // The page shows that there is one, and never what it is.
        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("AKIAEXAMPLE"),
            "the key id is not a secret: {body}"
        );
        assert!(
            !body.contains("s3cr3t-from-the-provider"),
            "the secret is never rendered back"
        );
        assert!(
            body.contains(r#"action="/node/backups/credentials/forget""#),
            "and it can be removed: {body}"
        );
    }

    /// Changing the region does not mean retyping the secret.
    ///
    /// The field renders empty whatever is stored — it is never shown
    /// again — so empty on save has to mean "keep it". Reading it as
    /// "clear it" would wipe the credential of everybody who came to fix
    /// a region, and they would find out at the next backup.
    #[tokio::test]
    async fn an_empty_secret_keeps_the_one_that_is_stored() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($secret:expr, $region:expr) => {
                console
                    .harness
                    .post("/node/backups/credentials")
                    .header("cookie", &cookie)
                    .form(&[
                        ("access_key_id", "AKIAEXAMPLE"),
                        ("secret_access_key", $secret),
                        ("region", $region),
                        ("endpoint", ""),
                    ])
                    .send()
                    .await
            };
        }

        save!("kept-across-the-edit", "us-east-1");
        let moved = save!("", "eu-central-1");
        assert_eq!(moved.header("location"), Some("/node/backup"));

        let stored = console.config.s3().expect("still a credential");
        assert_eq!(stored.secret_access_key, "kept-across-the-edit");
        assert_eq!(stored.region, "eu-central-1");

        // With nothing stored there is nothing to keep, and the refusal
        // says so rather than writing a credential with no secret in it.
        let bare = Console::new().await;
        let bare_cookie = bare.signed_in().await;
        let refused = bare
            .harness
            .post("/node/backups/credentials")
            .header("cookie", &bare_cookie)
            .form(&[
                ("access_key_id", "AKIAEXAMPLE"),
                ("secret_access_key", ""),
                ("region", "us-east-1"),
                ("endpoint", ""),
            ])
            .send()
            .await;
        assert!(
            refused
                .header("location")
                .unwrap_or_default()
                .contains("error="),
            "{:?}",
            refused.header("location")
        );
    }

    /// Forgetting the credential while the backups go to a bucket is
    /// refused.
    ///
    /// The button is disabled too, but that is a courtesy — the check
    /// that counts is here. A plan that cannot be carried out fails at
    /// the next slot, which is hours after the click that caused it.
    #[tokio::test]
    async fn a_credential_in_use_is_not_forgotten_by_accident() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/node/backups/credentials")
            .header("cookie", &cookie)
            .form(&[
                ("access_key_id", "AKIAEXAMPLE"),
                ("secret_access_key", "a-secret"),
                ("region", "us-east-1"),
                ("endpoint", ""),
            ])
            .send()
            .await;
        // The destination is written directly: saving it through the form
        // would ask the bucket, and there is no bucket in a test.
        crate::node::backups::store(
            &console.database,
            &crate::node::backups::Plan {
                destination: Some("s3://somebodys-bucket/nodes".into()),
                ..crate::node::backups::Plan::default()
            },
        )
        .await
        .expect("store the plan");

        let refused = console
            .harness
            .post("/node/backups/credentials/forget")
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(
            refused
                .header("location")
                .unwrap_or_default()
                .contains("error="),
            "{:?}",
            refused.header("location")
        );
        assert!(console.config.s3().is_some(), "still stored");

        // Pointed somewhere else, it goes.
        crate::node::backups::store(&console.database, &crate::node::backups::Plan::default())
            .await
            .expect("store the plan");
        console
            .harness
            .post("/node/backups/credentials/forget")
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(console.config.s3().is_none(), "and it is gone");
    }

    /// A failure is a value somebody can act on.
    ///
    /// A destination that has been refusing for a week is invisible in a
    /// journal nobody reads, so the reason is on the row — and from there
    /// on the page and in the attention card.
    #[tokio::test]
    async fn the_last_failure_is_on_the_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        crate::node::backups::record(
            &console.database,
            crate::clock::now_ms(),
            &Err("ssh: connect to host backups.example port 22: no route to host".into()),
        )
        .await
        .expect("record");

        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("no route to host"), "{body}");
    }

    /// Settings adjusts; it does not report.
    ///
    /// The meters were on this page when it was `/nodes`, and Jorge asked
    /// twice for them to go: nothing on them is adjustable, and the same
    /// readings are on the node's own page in the network. The second ask
    /// came with a screenshot, and this is the assertion that was missing
    /// — I had checked the source and told him it was already done, which
    /// is not the same as knowing.
    #[tokio::test]
    async fn the_settings_page_has_no_meters_on_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        // **Every tab, not `/node`.** That path is a redirect now, and a
        // redirect's body contains no meter — so asking it would pass
        // whatever the tabs held, which is the shape that has bitten this
        // session twice already.
        let mut bodies = Vec::new();
        for tab in NodeTab::all() {
            bodies.push(
                console
                    .harness
                    .get(&tab.path())
                    .header("cookie", &cookie)
                    .send()
                    .await
                    .body,
            );
        }
        for body in &bodies {
            // The meter's own markup, not the word: `Memory` is a label a
            // future card could reuse, while `data-meter` and `data-bar`
            // are what a bar *is*.
            assert!(
                !body.contains("data-meter"),
                "a meter is on the settings page: {body}"
            );
            assert!(!body.contains(r#"data-bar="#), "and so is a bar: {body}");
        }
        // What it is for is still here, each on its own tab.
        assert!(
            bodies
                .iter()
                .any(|body| body.contains("/node/capabilities")),
            "the capability switch is on none of them"
        );
        assert!(
            bodies.iter().any(|body| body.contains("/node/certificate")),
            "the certificate is on none of them"
        );

        // And the readings are on the node's page, which is where they
        // belong — so this is a move and not a deletion.
        let node = console
            .harness
            .get(&console.node_path)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            node.contains("data-meter"),
            "the readings went nowhere: {node}"
        );
    }

    /// The network is where you look; `/node` is what you change.
    ///
    /// They were one page called `/nodes`, so "how are my nodes" and
    /// "what does this machine do" had the same URL and the same word in
    /// the nav — and adding a node meant minting a token on one page and
    /// watching for it on the same one. Asked for by Jorge, who was not
    /// sure of the shape; this is the shape.
    ///
    /// The two old paths still answer, because a bookmark is somebody's
    /// and the plural named the list.
    #[tokio::test]
    async fn looking_at_the_network_and_configuring_this_node_are_two_places() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! get {
            ($path:expr) => {
                console
                    .harness
                    .get($path)
                    .header("cookie", &cookie)
                    .send()
                    .await
            };
        }

        // The old paths are redirects, and each goes where its name did.
        assert_eq!(
            get!("/nodes").header("location"),
            Some("/network"),
            "the plural named the list"
        );
        assert_eq!(
            get!("/settings").header("location"),
            Some("/node"),
            "and the gear lands on this machine"
        );

        let network = get!("/network").body;
        // The list, and the two things that change what is on it — in the
        // side nav, which is where this area's own column was empty. They
        // were buttons in the page's header first, and an empty sidebar
        // beside them was the thing Jorge saw.
        let sidebar = network
            .split_once(r#"<div class="side-inner">"#)
            .map(|(_, rest)| rest.split("</aside>").next().unwrap_or_default())
            .expect("a sidebar");
        assert!(
            sidebar.contains("/network/invite") && sidebar.contains("/network/join"),
            "the network area's column is empty: {sidebar}"
        );
        // And none of the switches: those are settings.
        assert!(
            !network.contains("/node/capabilities"),
            "the capability switch is on the list: {network}"
        );
        assert!(
            !network.contains("/node/certificate"),
            "and so is the certificate: {network}"
        );

        // `/node` is a redirect to its first tab, and the two forms are on
        // two of them since Jorge asked for tabs here as well.
        assert_eq!(get!("/node").header("location"), Some("/node/domain"));
        let certificate = get!("/node/domain").body;
        assert!(certificate.contains("/node/certificate"), "{certificate}");
        let allows = get!("/node/allows").body;
        assert!(allows.contains("/node/capabilities"), "{allows}");
        // Nothing that adds a node: these pages are about the one machine.
        for body in [&certificate, &allows] {
            assert!(
                !body.contains("/network/enrol"),
                "the invite form is in settings: {body}"
            );
        }

        // Each form is on the page it comes back to. A save that lands
        // somewhere else is a save that hides what it did.
        //
        // The invite *form* is asserted by
        // `a_node_with_no_address_explains_why_it_cannot_enrol` and not
        // here, because this node has neither an endpoint nor a domain —
        // so `enrol_card` deliberately renders the refusal instead. What
        // this pins is that the page is the card's home.
        assert!(get!("/network/invite").body.contains("Invite a node"));
        assert!(get!("/network/join").body.contains("name=\"token\""));
    }

    #[tokio::test]
    async fn the_list_shows_this_node() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/network")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Network"), "{body}");
        assert!(
            body.contains(&console.node_path),
            "the node links to itself: {body}"
        );
    }

    #[tokio::test]
    async fn a_node_page_shows_the_breakdown() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get(&console.node_path)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        for label in ["wabot-deploy", "containerd", "shims", "containers"] {
            assert!(body.contains(label), "{label} is missing from:\n{body}");
        }
    }

    #[tokio::test]
    async fn an_unknown_node_goes_back_to_the_list() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .get("/network/somewhere-else")
            .header("cookie", &cookie)
            .send()
            .await;
        response.assert_status(StatusCode::FOUND);
        assert_eq!(response.header("location"), Some("/network"));
    }

    /// The point of the whole form: a node installed before its DNS
    /// was ready serves its own certificate, and there has to be a way
    /// back from that without editing a file and restarting.
    ///
    /// The button says "Save domain" now. It used to say "Request a
    /// certificate", which described one thing it sometimes did rather
    /// than the thing it always does — and on a node with no domain,
    /// which is this one, no request was possible at all.
    #[tokio::test]
    async fn the_node_page_offers_to_save_a_domain() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/node/domain")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Save domain"), "{body}");
        assert!(
            !body.contains("Request a certificate"),
            "and no longer promises a request: {body}"
        );
        assert!(
            body.contains("/node/certificate"),
            "the form posts somewhere"
        );
    }

    /// Every `data-when` has to name a control that exists in the page
    /// it is on. A typo in the condition is the failure mode of a
    /// declarative island: nothing throws, the field is simply hidden
    /// forever or shown forever, and the page looks fine.
    pub(crate) fn conditions_name_real_controls(body: &str) {
        let mut checked = 0;
        for attribute in ["data-when=\"", "data-required-when=\""] {
            for (index, _) in body.match_indices(attribute) {
                let rest = &body[index + attribute.len()..];
                let condition = &rest[..rest.find('"').expect("a closed attribute")];
                let name = condition.split('=').next().expect("a name");
                assert!(
                    body.contains(&format!("name=\"{name}\"")),
                    "{condition} names no control on this page"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no conditional fields on this page");
    }

    /// The bug this island replaced: an inline `<script>` in the view
    /// body never runs when a boosted navigation swaps it in with
    /// `innerHTML`, so arriving here by clicking a link left a page
    /// that looked live and was a snapshot. An island host is data the
    /// runtime re-hydrates after every swap.
    #[tokio::test]
    async fn the_node_page_still_comes_alive_when_it_is_navigated_to() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let fragment = console
            .ui
            .with_header("cookie", cookie)
            .navigate(&console.node_path)
            .await;

        let html = fragment.html();
        assert!(
            html.contains("data-island=\"node-live\""),
            "the fragment carries the host: {html}"
        );
        assert!(
            !html.contains("<script"),
            "and nothing that a swap would silently not run: {html}"
        );
    }

    /// The island hides; it does not build. Both paths are in the HTML
    /// the node produced, so the form works with scripting off — that
    /// is the whole of the rule this console kept when it stopped
    /// saying "no JavaScript".
    #[tokio::test]
    async fn the_certificate_paths_are_rendered_and_merely_hidden() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/node/domain")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            body.contains("name=\"cert_path\""),
            "server-rendered: {body}"
        );
        assert!(body.contains("name=\"key_path\""), "{body}");
        assert!(
            body.contains("data-when=\"renew_with=file\""),
            "and conditioned: {body}"
        );
        conditions_name_real_controls(&body);
    }

    /// A node with no domain has nothing a public authority could
    /// certify, and that is a *state*, not a failure. It used to be
    /// indistinguishable from a node whose request had been refused,
    /// because only failures were recorded.
    #[tokio::test]
    async fn a_node_with_no_domain_says_so_rather_than_nothing() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/node/domain")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Self-signed"), "the badge: {body}");
        assert!(
            body.contains("no name an authority could"),
            "and what it means: {body}"
        );
    }

    /// Each state is one badge word, so the stream can replace it by
    /// assigning text. A state that rendered differently from the way
    /// it streams would be right until the first update.
    #[test]
    fn every_state_has_a_word_and_a_colour() {
        let states = [
            CertificateState::Local,
            CertificateState::Nameless,
            CertificateState::Working,
            CertificateState::Waiting,
            CertificateState::Trusted,
            CertificateState::Staging,
            CertificateState::Failed("refused".into()),
        ];
        let mut words = std::collections::BTreeSet::new();
        for state in &states {
            let (word, badge, dot) = state.badge();
            assert!(!word.is_empty() && !state.note().is_empty(), "{state:?}");
            assert!(
                badge.starts_with("badge") && dot.starts_with("dot"),
                "{state:?}"
            );
            words.insert(word);
        }
        assert_eq!(words.len(), states.len(), "two states read the same");
    }

    /// The reason a request is in flight has to reach the page while it
    /// is in flight — that is the whole point of the phase. Reading it
    /// from the last *error* could only ever describe attempts that had
    /// already finished badly.
    #[test]
    fn an_attempt_in_flight_outranks_what_was_stored() {
        let facts = super::super::CertificateFacts {
            issuer: "self-signed".into(),
            days_left: 0,
            trusted: false,
            source: crate::edge::certs::Source::SelfSigned,
        };
        let working = CertificateState::read(
            &facts,
            Some("node.example.com"),
            Some("refused last time".into()),
            crate::edge::acme::Phase::Working,
            false,
        );
        assert_eq!(working, CertificateState::Working);

        let settled = CertificateState::read(
            &facts,
            Some("node.example.com"),
            Some("refused last time".into()),
            crate::edge::acme::Phase::Idle,
            false,
        );
        assert_eq!(
            settled,
            CertificateState::Failed("refused last time".into())
        );

        // A node whose ACME is switched off is not "waiting" for
        // anything, and saying so would be an invitation to keep
        // looking at a page that will never change.
        let off = CertificateState::read(
            &facts,
            Some("node.example.com"),
            None,
            crate::edge::acme::Phase::Idle,
            true,
        );
        assert_eq!(off, CertificateState::Local);
    }

    /// A name that does not point here cannot be accepted: asking an
    /// authority to validate it spends one of five hourly attempts to
    /// be told what a lookup says for free.
    #[tokio::test]
    async fn a_name_that_does_not_resolve_here_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .post("/node/certificate")
            .header("cookie", &cookie)
            // `.invalid` is reserved by RFC 2606 precisely so it can
            // never resolve — no network needed to know the answer.
            .form(&[("domain", "nowhere.invalid")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default();
        assert!(
            location.starts_with(&format!("{}?error=", console.node_path)),
            "{location}"
        );

        assert_eq!(
            crate::node::settings::domain(&console.database, &Config::default()).await,
            None,
            "a refused name must not be stored"
        );
    }

    /// Clearing it is how somebody goes back to a node with no public
    /// name — and it must not be gated on a lookup.
    #[tokio::test]
    async fn the_domain_can_be_cleared() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        crate::node::settings::set_domain(&console.database, Some("was.example"))
            .await
            .expect("set");

        console
            .harness
            .post("/node/certificate")
            .header("cookie", &cookie)
            .form(&[("domain", "")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert_eq!(
            crate::node::settings::domain(&console.database, &Config::default()).await,
            None
        );
    }

    /// Changing what the whole node answers to is not something a
    /// member of one project gets to do.
    #[tokio::test]
    async fn only_an_admin_may_change_it() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        crate::node::settings::set_domain(&console.database, Some("was.example"))
            .await
            .expect("set");

        let response = console
            .harness
            .post("/node/certificate")
            .header("cookie", &member)
            .form(&[("domain", "")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
        assert_eq!(
            crate::node::settings::domain(&console.database, &Config::default())
                .await
                .as_deref(),
            Some("was.example"),
            "a member changed the node's domain"
        );
    }

    /// The stream is as private as the page it feeds.
    #[tokio::test]
    async fn the_stream_needs_a_session() {
        let console = Console::new().await;
        console.signed_in().await;

        let response = console
            .harness
            .get(&format!("{}/live", console.node_path))
            .send()
            .await;
        response.assert_status(StatusCode::UNAUTHORIZED);
    }

    /// A node with nowhere to be called back on cannot enrol anybody,
    /// and the page has to say which of the two it is rather than
    /// offering a form whose token would fail on the far machine.
    #[tokio::test]
    async fn a_node_with_no_address_explains_why_it_cannot_enrol() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/network/invite")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("no address another one could call back on"),
            "{body}"
        );
        assert!(
            !body.contains("/network/enrol"),
            "and offers no form: {body}"
        );

        // And the endpoint behind it is the boundary, not the form.
        let response = console
            .harness
            .post("/network/enrol")
            .header("cookie", &cookie)
            .form(&[("name", "alpine")])
            .send()
            .await;
        assert!(response
            .header("location")
            .is_some_and(|to| to.contains("error=")));
    }

    /// Mint a token the way the console does, and read it back out of
    /// the one page load that will ever show it.
    async fn enrol(console: &Console, cookie: &str, name: &str) -> String {
        crate::node::settings::set_domain(&console.database, Some("hub.example"))
            .await
            .expect("domain");
        crate::network::ensure_self(&console.database, &Config::default())
            .await
            .expect("this node");

        let response = console
            .harness
            .post("/network/enrol")
            .header("cookie", cookie)
            .form(&[("name", name)])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").expect("redirected");

        console
            .harness
            .get(location)
            .header("cookie", cookie)
            .send()
            .await
            .body
    }

    /// The whole enrolment surface: a token, shown once, with the
    /// command that spends it — and an address held for the node it
    /// was minted for.
    #[tokio::test]
    async fn a_join_token_is_shown_once_with_what_to_do_with_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = enrol(&console, &cookie, "alpine").await;
        let shown = body
            .split_once("wabot-deploy join ")
            .map(|(_, rest)| rest.split('<').next().unwrap_or_default().to_string())
            .expect("the command to run, with the token in it");
        assert!(shown.starts_with("wdj1."), "{body}");
        // **The name and the address are not here.** The token has a
        // page of its own now, with one thing on it; the list of what
        // has been minted stays on the page that minted them, which is
        // where the arrow goes back to.
        assert!(
            body.contains("/network/invite"),
            "no way back to the page that minted it: {body}"
        );

        // A second load of the same page shows nothing: the token is
        // stored hashed, and the nonce that named it is spent. Compared
        // against the token itself rather than the prefix — the join
        // form's placeholder carries that, and matching on it would be
        // a test that passes because it found the wrong thing.
        let again = console
            .harness
            .get("/network/invite")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(!again.contains(&shown), "the token came back: {again}");
        assert!(
            again.contains("Waiting"),
            "but the token is listed: {again}"
        );
    }

    /// **A node's name is its own.** It says what it is on the join
    /// callback, and that is the name the nodes list shows — so the
    /// mint form has nothing to ask for. It used to require one, which
    /// gave one machine two names differing by which table you read:
    /// the same fault the databases had, one floor up.
    ///
    /// What is left is a note for the operator with three unspent
    /// tokens, and a note is not something to be stopped by.
    #[tokio::test]
    async fn a_token_can_be_minted_without_naming_the_node() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = enrol(&console, &cookie, "").await;
        assert!(
            body.contains("wabot-deploy join "),
            "an unnamed token was refused: {body}"
        );

        let listed = console
            .harness
            .get("/network/invite")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(listed.contains("Waiting"), "{listed}");
        // The address is what identifies an unspent token, and it is
        // already in the row beside the empty note.
        assert!(listed.contains("10.42.0.2"), "{listed}");
    }

    /// Withdrawing gives the address back, so a token minted by mistake
    /// does not cost the overlay an address for ever.
    #[tokio::test]
    async fn a_token_can_be_withdrawn() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        enrol(&console, &cookie, "alpine").await;

        let enrolment = crate::network::enrolment::all(&console.database)
            .await
            .expect("list")
            .pop()
            .expect("one");
        console
            .harness
            .post(&format!("/network/enrolments/{}/withdraw", enrolment.id))
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(crate::network::enrolment::all(&console.database)
            .await
            .expect("list")
            .is_empty());
    }

    /// Putting a node on this one's network is not something a member
    /// of one project gets to do.
    #[tokio::test]
    async fn only_an_admin_may_enrol_a_node() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;
        crate::node::settings::set_domain(&console.database, Some("hub.example"))
            .await
            .expect("domain");
        crate::network::ensure_self(&console.database, &Config::default())
            .await
            .expect("this node");

        let response = console
            .harness
            .post("/network/enrol")
            .header("cookie", &member)
            .form(&[("name", "alpine")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
        assert!(
            crate::network::enrolment::all(&console.database)
                .await
                .expect("list")
                .is_empty(),
            "a member enrolled a node"
        );
    }

    /// A joined node's page is what is known about it, not this node's
    /// figures under somebody else's name. It has no memory reading and
    /// no certificate, and nothing has asked it anything.
    #[tokio::test]
    async fn a_joined_nodes_page_shows_what_is_known_and_no_more() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        joined(&console, "nd-elsewhere1").await;

        let body = console
            .harness
            .get("/network/nd-elsewhere1")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("10.42.0.9"), "its address: {body}");
        assert!(body.contains("on the overlay at"), "{body}");
        assert!(body.contains("Forget"), "and a way to stop listing it");
        assert!(
            !body.contains("Save domain") && !body.contains("data-cell=\"summary\""),
            "it rendered this node's own cards: {body}"
        );

        // It has a stream now, and only the errands are in it: a node
        // collects on a fifteen-second timer, so "waiting to be
        // collected" is exactly the badge somebody watches after
        // pressing the button. Memory and certificates stay behind
        // `mine` — those are answers only that machine can give, and
        // streaming this one's under its name would be invention.
        //
        // Asserted from the page rather than by calling `/live`: a
        // successful stream never ends, and a test that asks for one
        // waits for a body that will not come. That is what this test
        // did for an hour when the endpoint stopped answering 404 —
        // see `errands_are_reported_for_the_node_they_were_sent_to`
        // for the contents, which is the part worth pinning anyway.
        assert!(
            body.contains("data-island=\"node-live\""),
            "a joined node's page follows its errands: {body}"
        );
    }

    /// What the joined node's page shows about each instruction, and
    /// the reason it is tested here rather than through the stream: a
    /// stream that works never ends, so asking for one in a test is
    /// asking for a body that never arrives.
    #[tokio::test]
    async fn errands_are_reported_for_the_node_they_were_sent_to() {
        let console = Console::new().await;
        joined(&console, "nd-elsewhere1").await;

        network::errand::queue(
            &console.database,
            "nd-elsewhere1",
            network::errand::Kind::Host,
            &serde_json::json!({ "project": "shared" }),
        )
        .await
        .expect("queued");

        let cells = errand_cells(&console.database, "nd-elsewhere1").await;
        let (_, cell) = cells.iter().next().expect("one errand");
        assert_eq!(cell.word, "Waiting to be collected");

        assert!(
            errand_cells(&console.database, "nd-somebody-else")
                .await
                .is_empty(),
            "an errand belongs to the node it was sent to"
        );
    }

    #[tokio::test]
    async fn a_node_can_be_forgotten_from_its_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        joined(&console, "nd-elsewhere1").await;

        console
            .harness
            .post("/network/forget/nd-elsewhere1")
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert_eq!(
            crate::network::find(&console.database, "nd-elsewhere1")
                .await
                .expect("query"),
            None
        );
    }

    /// The half of joining that makes it not a one-way door, from the
    /// console of the node that granted it.
    #[tokio::test]
    async fn an_authority_can_be_revoked_from_this_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        crate::network::grant(&console.database, "nd-hub0000001", "a-secret")
            .await
            .expect("grant");

        let body = console
            .harness
            .get("/network")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Takes instructions from"), "{body}");
        assert!(body.contains("nd-hub0000001"), "{body}");

        console
            .harness
            .post("/network/revoke/nd-hub0000001")
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(
            !crate::network::is_authorised(&console.database, "nd-hub0000001").await,
            "it may still send this node errands"
        );
    }

    /// A row about another node describes the relationship, not the
    /// machine.
    ///
    /// This shipped in v0.3.0 and was found by looking at the two test
    /// nodes side by side: the Alpine node answers to a name and has a
    /// real certificate, so its own page said `public` — correctly —
    /// while at the same moment the hub listed it as `private`. Both
    /// rows were right and the pair was nonsense. The hub does not know
    /// what that machine is; it knows it has nothing but an overlay
    /// address for it, and that is what it says now.
    #[tokio::test]
    async fn a_joined_node_is_not_described_as_something_it_may_not_be() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        joined(&console, "nd-elsewhere1").await;

        let body = console
            .harness
            .get("/network")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        // Scoped to that node's own card: "Invite a node" is a
        // heading on this page, and an assertion over the whole body
        // would be one that passes or fails on the wrong text.
        let card = body
            .split_once("/network/nd-elsewhere1")
            .map(|(_, rest)| rest.split("</a>").next().unwrap_or_default())
            .expect("the node is listed");

        assert!(card.contains("on the overlay at 10.42.0.9"), "{card}");
        assert!(
            !card.contains("private"),
            "the hub called a machine private that calls itself public: {card}"
        );
    }

    /// The node somebody is looking at is the one every other row is
    /// compared against, and alphabetical order put it second on a list
    /// of two — which is how it was noticed.
    #[tokio::test]
    async fn this_node_is_the_first_card() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        // Sorts before this node's name, which has no domain and so is
        // named after the machine.
        joined(&console, "nd-elsewhere1").await;

        let body = console
            .harness
            .get("/network")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        let mine = body.find(&console.node_path).expect("this node is listed");
        let theirs = body.find("/network/nd-elsewhere1").expect("and the other");
        assert!(mine < theirs, "this node is not the first card");
    }

    /// The reason this form exists: a node that is already installed
    /// and already answering is one somebody is looking at through a
    /// browser, and joining must not send them to find its terminal.
    #[tokio::test]
    async fn the_nodes_page_offers_to_join_a_network() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/network/join")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Join a network"), "{body}");
        assert!(body.contains("/network/join"), "the form posts somewhere");
        assert!(body.contains("name=\"token\""), "{body}");
    }

    /// A refusal has to land on the page with its reason, and the token
    /// must not travel back with it — not into the query string, not
    /// into the form. It is the one secret this page handles.
    #[tokio::test]
    async fn a_join_that_is_refused_says_why_without_echoing_the_token() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .post("/network/join")
            .header("cookie", &cookie)
            .form(&[("token", "wdj1.not-a-real-token")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default();
        assert!(location.starts_with("/network/join?error="), "{location}");
        assert!(
            !location.contains("wdj1"),
            "the token came back in the URL: {location}"
        );

        // And nothing was granted on the strength of it.
        assert!(crate::network::authorities(&console.database)
            .await
            .expect("query")
            .is_empty());

        let body = console
            .harness
            .get(location)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("damaged"),
            "the reason reaches the page: {body}"
        );
        assert!(!body.contains("not-a-real-token"), "{body}");
    }

    /// Granting another node authority over this one is the largest
    /// thing anybody can do from this console.
    #[tokio::test]
    async fn only_an_admin_may_join_a_network() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        let response = console
            .harness
            .post("/network/join")
            .header("cookie", &member)
            .form(&[("token", "wdj1.whatever")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
    }

    /// A node that arrived through a join, as the API writes it.
    async fn joined(console: &Console, id: &str) {
        crate::network::save(
            &console.database,
            &crate::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: id.into(),
                name: "alpine.example".into(),
                kind: Kind::Private,
                endpoint: None,
                public_key: Some("0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: Some(super::super::now_ms()),
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("save");
    }

    #[tokio::test]
    async fn the_nodes_pages_need_a_session() {
        let console = Console::new().await;
        console.signed_in().await;

        for path in [
            "/network",
            "/network/invite",
            "/network/join",
            "/node",
            &console.node_path,
        ] {
            let response = console.harness.get(path).send().await;
            assert_eq!(
                response.header("location"),
                Some("/sign-in"),
                "{path} was served to nobody"
            );
        }
    }
}
