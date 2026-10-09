use hypertext::prelude::*;

use crate::domain::values::capability::Capability;
use crate::features::backups::node_backup_component;
use crate::features::node::texts::t;
use crate::infra::backup::plan::{Attempt, Plan};
use crate::infra::backup::s3::Credentials;
use crate::infra::backup::Manifest;
use crate::infra::edge::policy::Policy;
use crate::infra::node::memory::Snapshot;
use crate::shell::components::node_certificate::{CertificateCells, CertificateState};
use crate::shell::components::{node_capabilities, node_certificate, node_settings_cards};
use crate::shell::layouts::layout;

/// The four things this machine can be told about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTab {
    Domain,
    Allows,
    Resources,
    Backup,
}

impl NodeTab {
    pub fn slug(self) -> &'static str {
        match self {
            NodeTab::Domain => "domain",
            NodeTab::Allows => "allows",
            NodeTab::Resources => "resources",
            NodeTab::Backup => "backup",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            // The domain is what somebody comes here for; its certificate
            // is the consequence of having one, and the card inside still
            // says so.
            NodeTab::Domain => t("Domain"),
            NodeTab::Allows => t("Capabilities"),
            NodeTab::Resources => t("Resources"),
            NodeTab::Backup => t("Backup"),
        }
    }

    pub fn all() -> [NodeTab; 4] {
        [
            NodeTab::Domain,
            NodeTab::Allows,
            NodeTab::Resources,
            // Last, because it is the one somebody sets up once and then
            // comes back to only to read: what the other three answer is
            // "what is this node", and this one answers "and what happens
            // if it is gone".
            NodeTab::Backup,
        ]
    }

    pub fn parse(slug: &str) -> Option<Self> {
        Self::all().into_iter().find(|tab| tab.slug() == slug)
    }

    pub fn path(self) -> String {
        format!("/node/{}", self.slug())
    }
}

/// One tab's worth, and **only** that tab's.
///
/// A single struct with every field would have been twenty of them, four
/// fifths unread on any given render — and the handler would still have
/// had to fetch them all to fill it in. The backup tab's own comment
/// already said why that is not free: reading the list is a `read_dir`
/// and a `manifest.json` per backup, which the domain tab has no use for
/// on a page somebody opens because their node is unwell.
pub enum Panel {
    Domain {
        self_id: String,
        cells: CertificateCells,
        state: CertificateState,
        policy: Policy,
        domain: Option<String>,
        /// What the DNS check said. **Not the page's error**: the card
        /// used to take the controller's whole query struct and render
        /// the error a second time, under the one at the top.
        checked: Option<String>,
    },
    Allows {
        /// Every capability there is, so the form cannot be missing one.
        provides: Vec<(Capability, bool)>,
        /// An address the world could dial, whatever this node has
        /// decided to do with it. The difference between "cannot" and
        /// "will not" is the whole of what the switch says.
        reachable: bool,
        owns: bool,
        /// What it would have to let go of to stop being an owner. Said
        /// on the form rather than only in the refusal: a constraint
        /// somebody reads before clicking is worth more than the same
        /// sentence after.
        owned: usize,
    },
    Resources {
        snapshot: Snapshot,
        reserves: (Option<u64>, Option<u32>),
        cores: u32,
    },
    Backup {
        plan: Plan,
        attempt: Attempt,
        taken: Vec<(std::path::PathBuf, Manifest)>,
        /// Read from the file, like everything else that asks — and only
        /// the key and the endpoint are rendered. The secret goes in and
        /// never comes back out.
        s3: Option<Credentials>,
        archiving: bool,
        /// What an empty destination means, spelled out as the path it
        /// actually is. A placeholder saying "the default" would leave
        /// somebody looking for a directory whose name they were never
        /// told.
        default_destination: String,
        /// The file the credential card writes, named on the card. A
        /// secret stored somewhere the operator cannot find is a secret
        /// they cannot remove.
        config_path: String,
        self_id: String,
        running: bool,
    },
}

pub struct NodeSettings<'a> {
    pub tab: NodeTab,
    pub panel: Panel,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a NodeSettings<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <h1>(t("Node"))</h1>

        // No tagline. The tabs say what is here in four words, and a
        // sentence between the title and them was a line Jorge read once
        // and then had to scroll past on every visit — asked for and
        // removed.
        //
        // Links, the same shape a service's settings use.
        <nav class="tabs" aria-label=(t("Node"))>
            @for entry in NodeTab::all() {
                @if entry == page.tab {
                    <a href=(entry.path()) class="tab active"
                       aria-current="page">(entry.label())</a>
                } @else {
                    <a href=(entry.path()) class="tab">(entry.label())</a>
                }
            }
        </nav>

        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        @match &page.panel {
            Panel::Domain { self_id, cells, state, policy, domain, checked } => {
                // The certificate's own cells stream, so it keeps the
                // island — with only its half of the markup, which the
                // runtime tolerates: a cell it cannot find is skipped.
                // The meters are on the node's status page.
                (node_certificate::certificate_island(
                    self_id, cells, state, policy, domain.as_deref(), checked.as_deref()))
            }
            Panel::Allows { provides, reachable, owns, owned } => {
                (node_capabilities::capabilities_card(provides, *reachable, *owns, *owned))
            }
            Panel::Resources { snapshot, reserves, cores } => {
                (node_settings_cards::reserves_card(snapshot, *reserves, *cores))
            }
            Panel::Backup {
                plan, attempt, taken, s3, archiving, default_destination, config_path,
                self_id, running,
            } => {
                (node_backup_component::backup_card(plan, *archiving, default_destination))
                (node_backup_component::bucket_card(s3.as_ref(), config_path, plan))
                (node_backup_component::backup_state_card(plan, attempt, taken, self_id, *running))
            }
        }
    }
}
