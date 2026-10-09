//! How the node's TLS is currently answered for.
//!
//! A **shared DTO**: assembled in `console` out of the certificate store
//! and the config, read by the views, and carried by more than one page
//! — the node's domain tab and a service's hostname each ask the same
//! question about a different name.

use crate::infra::edge::certs::Source;

/// How long before expiry renewal starts. Here rather than in `console`
/// because `renews_in_days` is the only thing that reads it, and a
/// constant a hundred lines from its one use is a constant somebody
/// changes without finding it.
const RENEW_WINDOW_DAYS: i64 = 30;

/// How the node's TLS is currently answered for.
pub struct CertificateFacts {
    /// `letsencrypt`, `letsencrypt-staging`, `self-signed`, or whatever
    /// common name signed a certificate found on disk. A fact.
    pub issuer: String,
    pub days_left: i64,
    pub trusted: bool,
    /// How the node came by it, which is a different question — see
    /// migration `0012`.
    pub source: Source,
}

impl CertificateFacts {
    /// How long until renewal starts, or `None` when nothing will
    /// renew — a self-signed certificate reports zero days left, and
    /// subtracting the window from that is a negative number about an
    /// event that is not coming.
    /// What a name with nothing issued for it looks like: the node is
    /// serving it off the local authority's certificate, which is what
    /// `refresh_local` keeps covering every name.
    pub fn none() -> Self {
        Self {
            issuer: "self-signed".into(),
            days_left: 0,
            trusted: false,
            source: Source::SelfSigned,
        }
    }

    pub fn renews_in_days(&self) -> Option<i64> {
        let renews_in = self.days_left - RENEW_WINDOW_DAYS;
        (renews_in > 0).then_some(renews_in)
    }
}
