/// The name a project's registry should answer on.
///
/// `None` gives the project back the node's own name.
pub struct SetRegistryDomainRequest {
    pub domain: Option<String>,
}
