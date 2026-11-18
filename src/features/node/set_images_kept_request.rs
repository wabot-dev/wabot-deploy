/// How many images of each service this node keeps.
pub struct SetImagesKeptRequest {
    /// `None` is the default.
    pub kept: Option<usize>,
}
