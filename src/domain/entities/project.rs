//! A project: a workspace, and the services under it.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub created_at: i64,
    /// The node that asked for this, when it was not this one. `None`
    /// is ours — see migration `0021`.
    pub origin_node_id: Option<String>,
    /// The tenant it belongs to. `None` is the node's own — the same
    /// spelling the two above use, so a row written before tenants
    /// existed still means what it meant. See migration `0054`.
    pub tenant_id: Option<String>,
}

impl Project {
    /// Whether this node is the one that decides about it.
    ///
    /// A project that arrived on an errand is administered from the
    /// node that sent it: nothing here may change it, and the one thing
    /// this node's operator can always do is throw it out.
    ///
    /// Read by the capabilities form, which refuses to let a node stop
    /// being an owner while any of these are still here — a project of
    /// this node's own on a node that originates nothing would be
    /// neither editable nor evictable. `crate::domain::entities::service::Service::is_ours` is
    /// the same check one level down, and every service mutation goes
    /// through it.
    pub fn is_ours(&self) -> bool {
        self.origin_node_id.is_none()
    }

    /// Whether this is the machine's own rather than an organisation's.
    ///
    /// A question rather than a field read, so a controller that has no
    /// business knowing what an organisation is does not have to name
    /// one — and so deleting tenancy leaves this answering `true` for
    /// everything, which is what it meant before organisations existed.
    pub fn is_the_nodes_own(&self) -> bool {
        self.tenant_id.is_none()
    }
}
