/// A key that works, and the one moment its secret is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketKeyCreatedResponse {
    pub id: String,
    pub access_key_id: String,
    pub secret_key: String,
}
