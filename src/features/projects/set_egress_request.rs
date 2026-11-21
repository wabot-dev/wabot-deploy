use crate::domain::values::destinations::Destinations;

/// Where a project's containers may connect past its own network, as a
/// whole list. Empty lets nothing out.
pub struct SetEgressRequest {
    pub destinations: Destinations,
}
