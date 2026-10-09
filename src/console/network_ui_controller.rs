//! The network's pages: the nodes this one knows, and the two ways in.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::network::{self};
use crate::policies::access;

use super::auth::{signed_in, SessionMiddleware};
use super::ConsoleState;
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};

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

#[injectable]
pub struct NetworkUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl NetworkUiController {
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
        let may_enrol = nodes.iter().any(|node| node.is_self && node.may_be_edge());
        let now = super::now_ms();

        layout::head("Invite a node");
        let frame = Frame::new(&account, Area::Network, &projects, None, "/network/invite");
        let page = crate::views::pages::node_user_invite_node_page::InviteNode {
            nodes: &nodes,
            enrolments: &enrolments,
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
            error: query.error.as_deref(),
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
