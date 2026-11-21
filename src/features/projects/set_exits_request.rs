/// The nodes a project leaves for the internet through, by id. Empty is
/// each copy through its own node.
pub struct SetExitsRequest {
    pub exits: Vec<String>,
}
