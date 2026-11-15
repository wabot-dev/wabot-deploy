//! The nodes this one knows: the list, each one's pages, and the two
//! ways onto a network.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::domain::entities::account::Account;
use crate::features::network::node_connections_component::Links;
use crate::infra::network::{self, Node};

use crate::shell::auth::{signed_in, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Frame, Section};
use crate::shell::state::ConsoleState;

#[derive(Debug, Deserialize, Validate)]
pub struct NodePage {
    pub node: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct NodesPage {
    pub error: Option<String>,
    /// The node this one has just joined. A name, not a secret, so the
    /// query string is the right place for it — the same rule
    /// `back_with_error` follows.
    pub joined: Option<String>,
    /// Names a pasted token waiting to be read *once*, so its terms can
    /// be shown before it is spent. The nonce travels, the token does
    /// not — see `ConsoleState::reveals`.
    pub review: Option<String>,
}

/// Where a page of the old network area is now, with the refusal it was
/// carrying.
fn moved(to: &str, error: Option<&str>) -> ViewOutcome {
    Redirect::found(crate::shell::auth::with_error(to, error)).into()
}

#[injectable]
pub struct NetworkUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

impl NetworkUiController {
    /// Whoever runs the machine, or where to send anybody else.
    ///
    /// The node belongs to whoever runs it. Hiding the link is courtesy;
    /// this is the boundary.
    fn operator(&self) -> Result<Account, ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Err(Redirect::found("/sign-in").into());
        };
        match crate::domain::authz::node_user::manages_the_machine(&account) {
            true => Ok(account),
            false => Err(Redirect::found("/").into()),
        }
    }

    /// The node a path names, or the list it would have been on.
    async fn node_at(&self, id: &str) -> UiResult<Result<Node, ViewOutcome>> {
        Ok(match network::find(&self.state.database, id).await? {
            Some(node) => Ok(node),
            None => Err(Redirect::found("/nodes").into()),
        })
    }

    async fn orders_for(&self, node: &Node) -> UiResult<Vec<network::errand::Record>> {
        Ok(network::errand::all(&self.state.database)
            .await?
            .into_iter()
            .filter(|record| record.node_id == node.id)
            .collect())
    }

    /// A token pasted a moment ago, held for exactly one render so its
    /// terms can be read before it is spent. Taken, not copied: a reload
    /// asks for it again, which is the honest behaviour for a secret.
    fn reviewing(&self, nonce: Option<&str>) -> Option<(String, network::token::JoinToken)> {
        nonce
            .and_then(|nonce| self.state.reveals.take(nonce))
            .and_then(|token| {
                network::token::JoinToken::decode(&token)
                    .ok()
                    .map(|terms| (token, terms))
            })
    }
}

#[ui_controller("/", app)]
impl NetworkUiController {
    #[view("/network")]
    #[middleware(SessionMiddleware)]
    async fn network_moved(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        Ok(moved("/nodes", query.error.as_deref()))
    }

    #[view("/network/invite")]
    #[middleware(SessionMiddleware)]
    async fn invite_moved(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        Ok(moved("/nodes/invite", query.error.as_deref()))
    }

    #[view("/network/join")]
    #[middleware(SessionMiddleware)]
    async fn join_moved(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        Ok(moved("/nodes/join", query.error.as_deref()))
    }

    #[view("/network/:node")]
    #[middleware(SessionMiddleware)]
    async fn node_moved(&self, query: NodePage) -> UiResult<ViewOutcome> {
        Ok(moved(
            &format!("/nodes/{}", query.node),
            query.error.as_deref(),
        ))
    }

    #[view("/nodes")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(elsewhere) => return Ok(elsewhere),
        };

        let nodes = network::all(&self.state.database).await?;
        let paths = network::tunnel::paths(&self.state.database).await;
        let authorities = network::authorities(&self.state.database).await?;
        // What each of them was actually agreed to, which is the whole
        // point of the terms screen: a row saying only "takes
        // instructions from" would hide the difference between a node
        // that may do everything here and one that may do nothing.
        let mut agreed: Vec<(String, Vec<crate::domain::values::capability::Capability>)> =
            Vec::new();
        for authority in &authorities {
            agreed.push((
                authority.node_id.clone(),
                network::capability::granted_to(&self.state.database, &authority.node_id).await,
            ));
        }
        let reviewing = self.reviewing(query.review.as_deref());

        layout::head("Nodes");
        let frame = Frame::load(&self.state.database, &account, Section::Nodes, "/nodes").await;
        let page = crate::features::network::node_user_nodes_page::Nodes {
            nodes: &nodes,
            links: Links {
                nodes: &nodes,
                paths: &paths,
            },
            authorities: &authorities,
            agreed: &agreed,
            reviewing: reviewing.as_ref(),
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::network::node_user_nodes_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// Mint a token for a node that is not here yet.
    ///
    /// Its own page because it is the consequential half of the network:
    /// a form that hands out an authority over this machine has no
    /// business half-way down a list somebody opened to check on things.
    #[view("/nodes/invite")]
    #[middleware(SessionMiddleware)]
    async fn invite(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(elsewhere) => return Ok(elsewhere),
        };

        let nodes = network::all(&self.state.database).await?;
        let enrolments = network::enrolment::all(&self.state.database).await?;
        let may_enrol = nodes.iter().any(|node| node.is_self && node.may_be_edge());
        let now = crate::shell::state::now_ms();

        layout::head("Invite a node");
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Nodes,
            "/nodes/invite",
        )
        .await;
        let page = crate::features::network::node_user_invite_node_page::InviteNode {
            nodes: &nodes,
            enrolments: &enrolments,
            may_enrol,
            now,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::network::node_user_invite_node_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// Spend a token minted somewhere else.
    ///
    /// The terms screen lands here too: a token pasted is held for exactly
    /// one render so what it asks for can be read *before* it is spent.
    /// Terms shown after committing are a consent screen for a decision
    /// already made.
    #[view("/nodes/join")]
    #[middleware(SessionMiddleware)]
    async fn join_page(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(elsewhere) => return Ok(elsewhere),
        };

        let reviewing = self.reviewing(query.review.as_deref());

        layout::head("Join a network");
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Nodes,
            "/nodes/join",
        )
        .await;
        let page = crate::features::network::node_user_join_network_page::JoinNetwork {
            reviewing: reviewing.as_ref(),
            joined: query.joined.as_deref(),
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::network::node_user_join_network_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    #[view("/nodes/:node")]
    #[middleware(SessionMiddleware)]
    async fn node(&self, query: NodePage) -> UiResult<ViewOutcome> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(elsewhere) => return Ok(elsewhere),
        };
        let node = match self.node_at(&query.node).await? {
            Ok(node) => node,
            Err(elsewhere) => return Ok(elsewhere),
        };
        let path = format!("/nodes/{}", node.id);

        layout::head(&node.name);
        let frame = Frame::load(&self.state.database, &account, Section::Nodes, path).await;

        // A node that is not this one has no memory reading and no
        // certificate: both are answers only the machine itself can
        // give. So the page says what is known rather than rendering
        // cards full of "unknown".
        if !node.is_self {
            let orders = self.orders_for(&node).await?;
            let page = crate::features::network::node_user_remote_node_page::RemoteNode {
                node: &node,
                orders: &orders,
                error: query.error.as_deref(),
            };
            let body = crate::shell::texts::language::scoped(account.language, || {
                crate::features::network::node_user_remote_node_page::render(&page)
                    .render()
                    .into_inner()
            });
            return Ok(frame.render(body).into_view().into());
        }

        let snapshot = self.state.deployer.memory().await;
        let committed = self.state.deployer.committed().await;
        let reserves = (
            crate::infra::node::settings::memory_reserve(&self.state.database).await,
            crate::infra::node::settings::cpu_reserve(&self.state.database).await,
        );
        // Every part of it, which is three directory walks — so once per
        // render and never on the stream beside it: the memory tick is
        // every two seconds, and walking containerd's tree that often
        // would put this node at the top of its own CPU column.
        let disk = crate::infra::node::disk::breakdown(
            &self.state.config.node.data_dir,
            std::path::Path::new(crate::infra::node::disk::CONTAINERD_ROOT),
        );

        let page = crate::features::network::node_user_this_node_page::ThisNode {
            node: &node,
            disk: &disk,
            committed: &committed,
            snapshot: &snapshot,
            reserves,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::network::node_user_this_node_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    #[view("/nodes/:node/connections")]
    #[middleware(SessionMiddleware)]
    async fn connections(&self, query: NodePage) -> UiResult<ViewOutcome> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(elsewhere) => return Ok(elsewhere),
        };
        let node = match self.node_at(&query.node).await? {
            Ok(node) => node,
            Err(elsewhere) => return Ok(elsewhere),
        };

        let nodes = network::all(&self.state.database).await?;
        let paths = network::tunnel::paths(&self.state.database).await;

        layout::head(&node.name);
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Nodes,
            format!("/nodes/{}/connections", node.id),
        )
        .await;
        let page = crate::features::network::node_user_node_connections_page::NodeConnections {
            node: &node,
            links: Links {
                nodes: &nodes,
                paths: &paths,
            },
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::network::node_user_node_connections_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// The release another node runs. This machine's own is at
    /// `/node/version`, with the notes and the history only it has.
    #[view("/nodes/:node/version")]
    #[middleware(SessionMiddleware)]
    async fn version(&self, query: NodePage) -> UiResult<ViewOutcome> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(elsewhere) => return Ok(elsewhere),
        };
        let node = match self.node_at(&query.node).await? {
            Ok(node) => node,
            Err(elsewhere) => return Ok(elsewhere),
        };
        if node.is_self {
            return Ok(moved("/node/version", query.error.as_deref()));
        }

        let orders = self.orders_for(&node).await?;
        // The releases somebody can ask that node for.
        //
        // **Only when it has agreed to be kept up to date.** Reading the
        // list is a call to GitHub — cached for fifteen minutes, and
        // still a call — and a page that cannot offer the form has no
        // use for it. A failure is a card with an empty list and a
        // sentence saying so, never a page that does not render.
        let may_manage = node
            .allows
            .contains(&crate::domain::values::capability::Capability::Manage);
        let (releases, catalogue_failed) = match may_manage {
            false => (Vec::new(), false),
            true => match self.state.catalogue.releases().await {
                Ok(releases) => (
                    releases
                        .into_iter()
                        // Not filtered by what *this* machine can
                        // install: the answer belongs to the node being
                        // asked, which resolves the tag itself.
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
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Nodes,
            format!("/nodes/{}/version", node.id),
        )
        .await;
        let page =
            crate::features::network::node_user_remote_node_version_page::RemoteNodeVersion {
                node: &node,
                releases: &releases,
                catalogue_failed,
                orders: &orders,
                error: query.error.as_deref(),
            };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::network::node_user_remote_node_version_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// What this node decides about another one. This machine's own
    /// settings are at `/node`.
    #[view("/nodes/:node/settings")]
    #[middleware(SessionMiddleware)]
    async fn settings(&self, query: NodePage) -> UiResult<ViewOutcome> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(elsewhere) => return Ok(elsewhere),
        };
        let node = match self.node_at(&query.node).await? {
            Ok(node) => node,
            Err(elsewhere) => return Ok(elsewhere),
        };
        if node.is_self {
            return Ok(moved("/node", query.error.as_deref()));
        }

        // What this node has agreed to do for that one. Every
        // capability, so the form cannot be missing one.
        let held = network::capability::granted_to(&self.state.database, &node.id).await;
        let granted: Vec<(crate::domain::values::capability::Capability, bool)> =
            crate::domain::values::capability::Capability::ALL
                .into_iter()
                .map(|capability| (capability, held.contains(&capability)))
                .collect();

        layout::head(&node.name);
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Nodes,
            format!("/nodes/{}/settings", node.id),
        )
        .await;
        let page =
            crate::features::network::node_user_remote_node_settings_page::RemoteNodeSettings {
                node: &node,
                granted: &granted,
                error: query.error.as_deref(),
            };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::network::node_user_remote_node_settings_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}
