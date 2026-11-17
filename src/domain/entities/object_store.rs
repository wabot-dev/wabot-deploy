//! A store of buckets: the row beside a `garage` service holding what only
//! the engine has.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectStore {
    pub service_id: String,
    pub version: String,
    pub rpc_secret: String,
    pub admin_token: String,
    /// The name every bucket is a subdomain of. Garage is told it, so a
    /// request to `<bucket>.<domain>` reaches that bucket.
    pub domain: Option<String>,
    /// The backup a copy restored from one brings its volume from, until
    /// its buckets are rows again.
    pub restored_from: Option<String>,
}
