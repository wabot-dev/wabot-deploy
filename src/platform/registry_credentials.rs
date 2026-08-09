//! What this node presents to a registry that will not serve strangers.
//!
//! Keyed by the host in an image reference, because that is what the
//! pull path has to match against. See migration `0019`.

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{now_ms, PlatformResult};
use crate::runtime::images::Credential;

/// Remember how to authenticate to `host`.
///
/// Convergent: a second credential for the same registry replaces the
/// first. Two would mean the pull path picking one, and there is no
/// basis on which it could.
///
/// Nothing writes one yet. Accepting a `host` errand is what does — it
/// arrives carrying the authority's registry and a credential for it —
/// and the reading half is already wired into every deployment.
#[allow(dead_code)]
pub async fn set(
    database: &SqliteDatabase,
    host: &str,
    credential: &Credential,
) -> PlatformResult<()> {
    let (host, username, secret) = (
        host.to_string(),
        credential.username.clone(),
        credential.secret.clone(),
    );
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO registry_credential \
                   (\"host\", \"username\", \"secret\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (\"host\") DO UPDATE SET \
                   \"username\" = excluded.\"username\", \
                   \"secret\" = excluded.\"secret\", \
                   \"created_at\" = excluded.\"created_at\"",
                (host, username, secret, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// What to send when pulling `reference`, if anything.
///
/// `None` is the ordinary case and not a failure: most images come from
/// a registry that serves anybody, and a pull with no credential is how
/// every deployment worked before this existed.
pub async fn for_reference(database: &SqliteDatabase, reference: &str) -> Option<Credential> {
    let host = host_of(reference)?;
    let found: PlatformResult<Option<(String, String)>> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"username\", \"secret\" FROM registry_credential \
                     WHERE \"host\" = ?1",
                    [host],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
        })
        .await
        .map_err(Into::into);

    match found {
        Ok(found) => found.map(|(username, secret)| Credential { username, secret }),
        Err(error) => {
            // Reported and treated as absent. The pull then fails with
            // the registry's own 401, which names the registry — a
            // better thing to be looking at than a storage error.
            tracing::warn!(%error, "could not read the credential for a registry");
            None
        }
    }
}

/// The registry an image reference names, if it names one.
///
/// The rule every OCI client uses, and it is a heuristic rather than a
/// grammar. Two conditions, and the second is the one that is easy to
/// forget: there has to *be* a path after the first component, and that
/// component has to look like a host — a dot, a colon, or exactly
/// `localhost`.
///
/// Without the first condition `alpine:3.23` reads as a registry called
/// `alpine` on port `3.23`, because a tag's colon looks exactly like a
/// port's. That shipped for an hour in two copies of this rule, which is
/// why there is now one and it is public.
///
/// `library/alpine` is not a host called `library` either — that is the
/// second condition doing its job.
pub fn host_of(reference: &str) -> Option<String> {
    let (first, _) = reference.split_once('/')?;
    let looks_like_a_host = first.contains('.') || first.contains(':') || first == "localhost";
    looks_like_a_host.then(|| first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential() -> Credential {
        Credential {
            username: "wabot".into(),
            secret: "a-push-token".into(),
        }
    }

    #[tokio::test]
    async fn a_credential_is_found_by_the_host_in_the_reference() {
        let database = crate::db::open_in_memory().await.expect("open");
        set(&database, "hub.example.com", &credential())
            .await
            .expect("set");

        assert_eq!(
            for_reference(&database, "hub.example.com/proj/app@sha256:abc").await,
            Some(credential())
        );
        assert_eq!(
            for_reference(&database, "elsewhere.example/proj/app:v1").await,
            None,
            "a credential for one registry was sent to another"
        );
    }

    /// Two credentials for one registry would mean the pull path
    /// choosing, and there is no basis on which it could.
    #[tokio::test]
    async fn a_second_credential_replaces_the_first() {
        let database = crate::db::open_in_memory().await.expect("open");
        set(&database, "hub.example.com", &credential())
            .await
            .expect("set");
        let rotated = Credential {
            username: "wabot".into(),
            secret: "a-newer-token".into(),
        };
        set(&database, "hub.example.com", &rotated)
            .await
            .expect("set");

        assert_eq!(
            for_reference(&database, "hub.example.com/proj/app:v1").await,
            Some(rotated)
        );
    }

    /// `library/alpine` is not a host called `library`. Getting this
    /// wrong would send a node's credential to Docker Hub.
    #[test]
    fn a_namespace_is_not_a_registry() {
        assert_eq!(
            host_of("hub.example.com/proj/app:v1").as_deref(),
            Some("hub.example.com")
        );
        assert_eq!(
            host_of("localhost:5000/proj/app").as_deref(),
            Some("localhost:5000")
        );
        assert_eq!(host_of("localhost/proj/app").as_deref(), Some("localhost"));

        assert_eq!(host_of("library/alpine:3.23"), None);
        assert_eq!(host_of("alpine"), None);
        assert_eq!(host_of(""), None);

        // The one that got away: a tag's colon looks exactly like a
        // port's, so a reference with no path at all read as a registry
        // called `alpine` on port `3.23`.
        assert_eq!(host_of("alpine:3.23"), None);
        assert_eq!(host_of("ubuntu:24.04"), None);
    }

    /// The ordinary case, and not a failure: most images come from a
    /// registry that serves anybody.
    #[tokio::test]
    async fn no_credential_is_a_normal_answer() {
        let database = crate::db::open_in_memory().await.expect("open");
        assert_eq!(for_reference(&database, "alpine:3.23").await, None);
        assert_eq!(
            for_reference(&database, "hub.example.com/proj/app:v1").await,
            None
        );
    }
}
