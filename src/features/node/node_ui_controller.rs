//! This machine's own settings.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::infra::network::{self};
use crate::infra::repositories::access;

use crate::features::node::node_user_node_settings_page::{NodeTab, Panel};
use crate::shell::auth::{signed_in, SessionMiddleware};
use crate::shell::components::node_certificate::CertificateState;
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Frame, Section};
use crate::shell::state::ConsoleState;

#[derive(Debug, Deserialize, Validate, Default)]
pub struct NodeSettingsPage {
    pub error: Option<String>,
    pub checked: Option<String>,
    /// Which tab, from the path. Absent on the bare `/node`, which sends
    /// you to the first one.
    pub tab: Option<String>,
}

#[injectable]
pub struct NodeUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl NodeUiController {
    /// Where whoever runs the machine lands.
    ///
    /// Only theirs: a member of one project has no business being told
    /// about another's certificates, and every source here is node-wide.
    #[view("/status")]
    #[middleware(SessionMiddleware)]
    async fn status(&self, query: NodeSettingsPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/projects").into());
        }

        let concerns = crate::features::node::node_user_see_what_needs_attention::run(
            &self.state.database,
            &self.state.config,
            &self.state.deployer,
            &account,
        )
        .await
        .unwrap_or_default();
        let nodes = network::all(&self.state.database).await?;
        let paths = network::tunnel::paths(&self.state.database).await;
        let memory = self.state.deployer.memory().await;
        // The filesystem's own two numbers, and not the walk by part: that
        // is three directory trees, and this is the page somebody lands
        // on.
        let disk = crate::infra::node::disk::filesystem(&self.state.config.node.data_dir);
        let domain = crate::infra::node::settings::domain(&self.state.database).await;

        layout::head("Status");
        let frame = Frame::load(&self.state.database, &account, Section::Status, "/status").await;
        let page = crate::features::node::node_user_status_page::Status {
            node: nodes.iter().find(|node| node.is_self),
            concerns: &concerns,
            memory: &memory,
            disk,
            domain: domain.as_deref(),
            links: crate::features::network::node_connections_component::Links {
                nodes: &nodes,
                paths: &paths,
            },
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::node::node_user_status_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// Where the gear used to land. There is no settings area now — what
    /// can be changed about this machine is a tab of its own page — so a
    /// link kept from before goes there.
    #[view("/settings")]
    #[middleware(SessionMiddleware)]
    async fn settings(&self) -> UiResult<ViewOutcome> {
        match signed_in(&self.auth)
            .is_some_and(|account| crate::domain::authz::node_user::manages_the_machine(&account))
        {
            true => Ok(Redirect::found("/node").into()),
            false => Ok(Redirect::found("/").into()),
        }
    }

    /// The bare path is not a page: it names the first tab.
    #[view("/node")]
    #[middleware(SessionMiddleware)]
    async fn this_node_root(&self, query: NodeSettingsPage) -> UiResult<ViewOutcome> {
        // Nobody signed in goes to sign in, not to the projects list: the
        // three answers are different and `the_nodes_pages_need_a_session`
        // asks every page in the area which one it gives.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        match account.is_admin() {
            true => Ok(Redirect::found(crate::shell::auth::with_error(
                &NodeTab::Domain.path(),
                query.error.as_deref(),
            ))
            .into()),
            false => Ok(Redirect::found("/").into()),
        }
    }

    #[view("/node/:tab")]
    #[middleware(SessionMiddleware)]
    async fn this_node(&self, query: NodeSettingsPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        // A word that is not a tab goes to the first one, which covers a
        // hand-typed URL and a link kept from before there were tabs.
        let Some(tab) = query.tab.as_deref().and_then(NodeTab::parse) else {
            return Ok(Redirect::found(NodeTab::Domain.path()).into());
        };

        let projects = access::projects_for(&self.state.database, &account).await?;
        let nodes = network::all(&self.state.database).await?;
        let Some(me) = nodes.iter().find(|node| node.is_self) else {
            return Ok(Redirect::found("/nodes").into());
        };
        let self_id = me.id.clone();

        // **Only what this tab draws.** All of it used to be read on
        // every render, and the cost is not theoretical: `backup::taken`
        // is a `read_dir` and a `manifest.json` per backup in the root,
        // and this is the page somebody opens because their node is
        // unwell. The comment saying so was already on one of them.
        let panel = match tab {
            NodeTab::Domain => {
                let domain = crate::infra::node::settings::domain(&self.state.database).await;
                let facts = crate::shell::state::certificate_facts(&self.state).await;
                let policy = crate::infra::edge::policy::for_name(
                    &self.state.database,
                    domain
                        .as_deref()
                        .unwrap_or(crate::infra::edge::certs::FALLBACK_NAME),
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
                        .or(crate::infra::node::settings::acme_error(&self.state.database).await),
                    self.state.certificates.phase(),
                    !crate::infra::node::settings::acme_enabled(&self.state.database).await,
                );
                let cells = crate::shell::components::node_certificate::certificate_cells(
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
                let domain = crate::infra::node::settings::domain(&self.state.database).await;
                let mut provides = Vec::new();
                for capability in crate::domain::values::capability::Capability::ALL {
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
                    buckets: crate::infra::node::settings::buckets_enabled(&self.state.database)
                        .await,
                }
            }
            NodeTab::Access => Panel::Access {
                console: crate::infra::repositories::allowlists::of(
                    &self.state.database,
                    &crate::domain::values::allowlist::Door::Console,
                )
                .await?,
                registry: crate::infra::repositories::allowlists::of(
                    &self.state.database,
                    &crate::domain::values::allowlist::Door::Registry,
                )
                .await?,
                ssh: crate::infra::repositories::allowlists::of(
                    &self.state.database,
                    &crate::domain::values::allowlist::Door::Ssh,
                )
                .await?,
                ssh_ports: crate::infra::firewall::ssh_ports().await,
                refused: crate::infra::firewall::last_error(&self.state.database).await,
            },
            NodeTab::Dns => {
                let table = self
                    .state
                    .container
                    .try_resolve::<crate::infra::dns::Zones>()
                    .ok();
                match crate::features::dns::node_user_see_dns::run(
                    &self.state.database,
                    table.as_deref(),
                    &account,
                )
                .await
                {
                    Ok(dns) => Panel::Dns { dns },
                    Err(_) => return Ok(Redirect::found("/").into()),
                }
            }
            NodeTab::Resources => Panel::Resources {
                // The reading, not a stream: this page does not host the
                // meters.
                snapshot: self.state.deployer.memory().await,
                reserves: (
                    crate::infra::node::settings::memory_reserve(&self.state.database).await,
                    crate::infra::node::settings::cpu_reserve(&self.state.database).await,
                ),
                cores: crate::infra::node::cpu::cores(),
            },
            NodeTab::Backup => Panel::Backup {
                plan: crate::infra::backup::plan::plan(&self.state.database).await,
                attempt: crate::infra::backup::plan::last(&self.state.database).await,
                taken: crate::infra::backup::taken(&self.state.config.node.data_dir),
                s3: self.state.config.s3(),
                archiving: crate::infra::node::settings::archiving(&self.state.database).await,
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
                    .unwrap_or_else(|| {
                        std::path::PathBuf::from(crate::infra::config::DEFAULT_CONFIG_PATH)
                    })
                    .display()
                    .to_string(),
                self_id,
                // A flag rather than a row, so the card stays a function
                // of values — see `node::backups::in_progress`.
                running: crate::infra::backup::plan::in_progress(),
            },
        };

        layout::head(&me.name);
        let frame = Frame::load(&self.state.database, &account, Section::Nodes, tab.path()).await;
        let page = crate::features::node::node_user_node_settings_page::NodeSettings {
            node: me,
            tab,
            panel,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::node::node_user_node_settings_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}
