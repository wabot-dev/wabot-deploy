//! A service: an image, a port, and what the operator wants it doing.
//!
//! ## Desired state is not observed state
//!
//! `desired_state` is what was asked for. What containerd reports is
//! what *is*. A service can be desired-running and crashed, and
//! collapsing the two loses the only field that says which — so the
//! console can never tell "stopped on purpose" from "fell over".

use std::collections::BTreeMap;

use serde::Serialize;

use crate::platform::{slugify, PlatformError, PlatformResult};
use crate::values::image_reference::ImageReference;
/// Where the node allocates host ports from.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    Running,
    Stopped,
}

impl DesiredState {
    pub fn as_str(self) -> &'static str {
        match self {
            DesiredState::Running => "running",
            DesiredState::Stopped => "stopped",
        }
    }

    pub(crate) fn parse(text: &str) -> Self {
        match text {
            "stopped" => DesiredState::Stopped,
            // Anything unrecognised reads as running: a row we cannot
            // interpret should keep serving rather than quietly stop.
            _ => DesiredState::Running,
        }
    }
}

/// What kind of thing this service is.
///
/// Not a hint. The deploy path reads it to decide what a container
/// needs — a volume, a ceiling, tuning arguments, a role — and the
/// console reads it to decide which page to show, because a managed
/// database has no image field and no environment editor: the node
/// writes both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An image somebody chose, run as it is. Everything, until there
    /// were databases.
    Container,
    Postgres,
    /// A kind from a newer node. Held and refused rather than parsed
    /// into a panic — the same shape `errand::Kind` uses, and for the
    /// same reason: a row outlives the version that wrote it.
    Unknown,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Container => "container",
            Kind::Postgres => "postgres",
            Kind::Unknown => "unknown",
        }
    }

    pub(crate) fn parse(text: &str) -> Self {
        match text {
            "container" => Kind::Container,
            "postgres" => Kind::Postgres,
            _ => Kind::Unknown,
        }
    }

    /// Whether the node writes this service's image, environment and
    /// arguments rather than the operator.
    ///
    /// The question every console page asks, and the one that must not
    /// be spelled `kind == Postgres` in a dozen places: a second engine
    /// would make every one of them wrong.
    pub fn is_managed(self) -> bool {
        !matches!(self, Kind::Container)
    }

    /// What a node has to have agreed to before a copy of this can be
    /// placed on it.
    ///
    /// **Holding somebody's data and running somebody's container are
    /// different favours**, each granted and withdrawn on its own, so
    /// this is one capability rather than two — the same answer
    /// `errand::consent_for` gives for the errand that carries the copy,
    /// and the same one `evict_ungranted` asks at boot before throwing
    /// one off.
    ///
    /// It is here because the picker that offers nodes and the dispatch
    /// that sends the errand must not disagree, and they did: the
    /// placement form filtered on `host` while a database copy travels as
    /// `Kind::Database` and needs `store`, so a node with one and not the
    /// other was offered, chosen, written as a row, and then skipped in
    /// silence — a copy the console showed as placed that was never sent
    /// anywhere.
    pub fn placement_capability(self) -> crate::network::capability::Capability {
        match self.is_managed() {
            true => crate::network::capability::Capability::Store,
            false => crate::network::capability::Capability::Host,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Service {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub slug: String,
    pub image: String,
    pub env: BTreeMap<String, String>,
    pub desired_state: DesiredState,
    pub last_error: Option<String>,
    /// Vestigial. An address belongs to a **replica** now — a service
    /// is *n* running copies and each has its own — so nothing writes
    /// this and nothing should read it. The column goes in a migration
    /// of its own, once no node is still running a version that
    /// selects it.
    pub address: Option<String>,
    /// The tag a push has to carry for this service to care. `None`
    /// means "whatever tag my image reference names".
    pub track_tag: Option<String>,
    /// Whether a push to that tag goes out on its own.
    pub auto_deploy: bool,
    /// The node that asked for this, when it was not this one. `None`
    /// is ours — see migration `0021`.
    pub origin_node_id: Option<String>,
    /// The most memory each of its containers may have, in bytes.
    ///
    /// `None` is no ceiling, which is what every container had before
    /// there were presets. A number here reaches the OCI spec as
    /// `memory.max` with swap turned off — see `runtime::spec`.
    pub memory_limit: Option<u64>,
    /// The most CPU its containers may have, in millicores. `None` is no
    /// ceiling, and **the ceiling is also the reservation** — see
    /// `migrations/0038_cpu_limit.sql` for why there is no second number
    /// for a request.
    pub cpu_millicores: Option<u32>,
    /// The most disk this service's volumes may have, per copy.
    ///
    /// **Admission, not containment**, and the difference is the whole
    /// of what this field is: there is no `disk.max` to write into a
    /// container's spec, so this stops the *next* placement and does not
    /// stop a container already running from filling the machine. The
    /// form has to say so — see migration `0055`.
    pub disk_limit: Option<u64>,
    /// Whether this service's output is timestamped as it is written.
    ///
    /// Off by default, and the default matters more than usual here:
    /// turning it on puts a reader between the container and its log, and
    /// a reader that stops reading blocks the container. See
    /// `commands::log_writer`.
    pub log_timestamps: bool,
    /// How much disk this service's logs may keep, per copy.
    ///
    /// `None` is the default, which is `logs::GENERATIONS` behind the live
    /// file. Not a number stored at creation: a stored default goes stale
    /// the day the constant moves and then says the opposite of what the
    /// node does.
    pub log_budget: Option<u64>,
    pub kind: Kind,
}

impl Service {
    /// A new service, which is where its rules are.
    ///
    /// Every default here is the *honest* one, and each is a decision:
    /// a service made from an errand gets its origin from `set_origin`,
    /// which only the collector calls, so forgetting it cannot silently
    /// make a foreign service editable. `Kind::Container` for the same
    /// reason — `crate::commands::create_database::create` says otherwise straight afterwards,
    /// and a missed call cannot silently make something managed.
    pub fn new(
        project_id: &str,
        name: &str,
        image: &str,
        env: &[(String, String)],
    ) -> PlatformResult<Self> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 {
            return Err(PlatformError::Refused(
                "a service name is between 1 and 60 characters".into(),
            ));
        }
        let slug = slugify(name);
        if slug.is_empty() {
            return Err(PlatformError::Refused(
                "that name has nothing a hostname can be built from".into(),
            ));
        }
        let image = ImageReference::parse(image)?;

        Ok(Self {
            id: format!("svc-{}", wabot::prelude::password::generate(12)),
            project_id: project_id.to_string(),
            name: name.to_string(),
            slug,
            image: image.to_string(),
            env: env.iter().cloned().collect(),
            desired_state: DesiredState::Running,
            last_error: None,
            address: None,
            track_tag: None,
            origin_node_id: None,
            // On by default. A node where CI has to be told twice — once
            // to push, once to deploy — is one where the second half gets
            // forgotten and somebody debugs a version that never went out.
            auto_deploy: true,
            // No ceiling, which is what every service had before there
            // were presets. Choosing one is `set_memory_limit`, and a
            // database picks one at creation.
            memory_limit: None,
            cpu_millicores: None,
            disk_limit: None,
            log_timestamps: false,
            log_budget: None,
            kind: Kind::Container,
        })
    }
}

impl Service {
    /// Whether this node is the one that decides about it.
    ///
    /// A service that arrived on an errand is administered from the
    /// node that sent it: nothing here may change it, and the one thing
    /// this node's operator can always do is throw it out. Two nodes
    /// disagreeing about one service is not a conflict anything can
    /// settle, which is the same reason a name belongs to one
    /// authority.
    pub fn is_ours(&self) -> bool {
        self.origin_node_id.is_none()
    }

    /// The containerd container id, and the label every container of
    /// this node carries.
    ///
    /// Derived rather than stored: it has to be reconstructible from
    /// the row alone, because reconciliation on boot starts from rows
    /// and asks containerd what it has.
    ///
    /// A dot joins them, for two reasons that agree. containerd
    /// validates ids against `^[A-Za-z0-9]+(?:[._-][A-Za-z0-9]+)*$`,
    /// which allows single separators only — `project--service` is
    /// refused outright, as this found out on a real node. And a slug
    /// is `[a-z0-9-]`, so a dot cannot occur inside either half: the
    /// id parses back apart unambiguously, which `-` would not.
    ///
    /// **Slot 1's**, which is why nothing calls this any more: a service is
    /// *n* copies, and asking a service for "its" container id answered
    /// about one of them. That is what made the memory reading count a
    /// three-copy service once. `Replica::container_id` is the one to ask.
    #[allow(dead_code)]
    pub fn container_id(&self, project_slug: &str) -> String {
        format!("{project_slug}.{}", self.slug)
    }
}

/// Point a service at a different image.
///
/// Separate from `create` because the reason to call it is that the
/// service already exists — an errand asking for a service that is
/// already there is convergent, and creating a second one under the
/// same name would be the retry making a mess rather than nothing.
/// Record that another node asked for this service.
///
/// Called by the collector and nothing else. Separate from `create` so
/// that the default is the honest one — a service is this node's until
/// something says otherwise, and forgetting to say so cannot silently
/// make a foreign service editable.
#[allow(dead_code)]
/// What a push has to carry, and whether it goes out on its own.
/// One service by its id.
///
/// What a caller holding an id has, as against `in_project`, which is
/// what a caller holding a URL has.
/// Record that the node manages this service rather than the operator.
///
/// Separate from `create` for the reason `set_origin` is: the default
/// is the honest one. A service that was a plain container for even a
/// moment is one the deploy path would have started without its
/// volume — an empty database that looks like it worked.
/// How much memory each of its containers may have.
///
/// Takes effect at the next deployment, like everything else in the OCI
/// spec: a cgroup limit is written when the container is created, and
/// nothing here reaches into a running one to change it. The page says
/// so rather than the operator finding out.
/// Cap how much CPU a service's containers may have, in millicores.
///
/// Refused below `presets::MIN_MILLICORES`, the same shape
/// `set_memory_limit` follows and for the same reason. This used to
/// require a rung, arguing that a field taking any number is a field
/// somebody types 50 into — which is a container that cannot finish
/// starting, and an argument for a floor rather than for a list.
///
/// Takes effect at the next deployment — a cgroup limit is written into
/// the spec when the container is created, and nothing here reaches into
/// a running one to change it.
/// The most disk this service's volumes may have, per copy.
///
/// **Admission, not containment.** Nothing writes this into a
/// container's spec, because there is nothing to write it into: volumes
/// share a filesystem and enforcing a share of one is XFS project quotas
/// or a loopback per volume, which is a different order of work. What
/// this number does is stop the *next* placement and charge an
/// organisation's budget.
///
/// The floor is the same as memory's, and for a plainer reason: a
/// database's own files are already megabytes before it has stored
/// anything, so a ceiling under that is a service that can never start.
/// Turn timestamped logging on or off for one service.
///
/// Takes effect at the next deployment, not now: the logging URI is
/// chosen when a container is created and containerd cannot change a
/// running container's stdio. The console says so on the switch, because
/// a setting that saves and appears to do nothing is one somebody clicks
/// twice.
impl Service {
    /// No image yet, so there is nothing to run.
    ///
    /// A service can be created without one and wait for the first push
    /// to its own repository — which is the ordinary way somebody
    /// deploys their own application, and used to mean typing a
    /// reference for an image that did not exist yet.
    ///
    /// Empty rather than `Option<String>`: fifty-odd places read this
    /// field, and a nullable column would have every one of them decide
    /// what absent means. One question, asked here.
    pub fn awaiting_push(&self) -> bool {
        self.image.trim().is_empty()
    }
}
