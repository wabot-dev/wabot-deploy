//! What this node presents to a registry that will not serve strangers.
//!
//! Keyed by the host in an image reference, because that is what the
//! pull path has to match against. See migration `0019`.

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use crate::domain::values::image_reference::ImageReference;

use crate::domain::errors::PlatformResult;
use crate::infra::clock::now_ms;
use crate::infra::runtime::images::Credential;

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
    let host = ImageReference::parse(reference).ok()?.host()?;
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
        let database = crate::infra::db::open_in_memory().await.expect("open");
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
        let database = crate::infra::db::open_in_memory().await.expect("open");
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

    /// The ordinary case, and not a failure: most images come from a
    /// registry that serves anybody.
    #[tokio::test]
    async fn no_credential_is_a_normal_answer() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        assert_eq!(for_reference(&database, "alpine:3.23").await, None);
        assert_eq!(
            for_reference(&database, "hub.example.com/proj/app:v1").await,
            None
        );
    }
}
