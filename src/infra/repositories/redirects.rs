//! Every `SELECT`, `INSERT` and `DELETE` a [`Redirect`] needs.

use wabot::sqlite::rusqlite::Row;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::port::Port;
use crate::domain::entities::redirect::Redirect;
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::values::hostname::{Hostname, WILDCARD_REFUSED};
use crate::infra::clock::now_ms;

pub const NO_TARGET: &str =
    "a redirect goes to one of this service's own hostnames, and that port has none";
pub const TO_ITSELF: &str = "a name cannot redirect to itself";
pub const IS_A_HOSTNAME: &str =
    "that name is a hostname a service answers on — a name either serves or redirects";
pub const ALREADY_REDIRECTS: &str = "that name already redirects somewhere";
pub const NO_SUCH_REDIRECT: &str = "there is no such redirect";

/// Send `hostname` to whatever `port` answers to, and answer for it where
/// `served_by` says — empty is this node, as for a port's own name.
pub async fn create(
    database: &SqliteDatabase,
    port: &Port,
    hostname: &str,
    served_by: &[String],
) -> PlatformResult<Redirect> {
    let Some(target) = port.hostname.clone() else {
        return Err(PlatformError::Refused(NO_TARGET.into()));
    };
    let parsed = Hostname::parse(hostname);
    if parsed.is_empty() {
        return Err(PlatformError::Refused("a redirect needs a name".into()));
    }
    if parsed.is_wildcard() {
        return Err(PlatformError::Refused(WILDCARD_REFUSED.into()));
    }
    let hostname = parsed.as_str().to_string();
    if hostname == target {
        return Err(PlatformError::Refused(TO_ITSELF.into()));
    }
    let serves = crate::infra::repositories::ports::all(database)
        .await?
        .into_iter()
        .any(|port| port.hostname.as_deref() == Some(hostname.as_str()));
    if serves {
        return Err(PlatformError::Refused(IS_A_HOSTNAME.into()));
    }

    let (name, port_id) = (hostname.clone(), port.id.clone());
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO service_redirect (\"hostname\", \"port_id\", \"created_at\") \
                 VALUES (?1, ?2, ?3)",
                (name, port_id, now_ms()),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| match error.to_string().contains("UNIQUE") {
            true => PlatformError::Refused(ALREADY_REDIRECTS.into()),
            false => PlatformError::Storage(error),
        })?;

    crate::infra::repositories::ports::serve_from(database, &port.service_id, &hostname, served_by)
        .await?;
    Ok(Redirect {
        hostname,
        port_id: port.id.clone(),
        service_id: port.service_id.clone(),
        target,
    })
}

/// Every redirect on the node whose port still has a name.
pub async fn all(database: &SqliteDatabase) -> PlatformResult<Vec<Redirect>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(&format!("{COLUMNS} ORDER BY r.\"hostname\""))?
                .query_map([], read)?
                .collect()
        })
        .await?)
}

pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<Redirect>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "{COLUMNS} AND p.\"service_id\" = ?1 ORDER BY r.\"hostname\""
                ))?
                .query_map([service_id], read)?
                .collect()
        })
        .await?)
}

/// Stop redirecting a name, and stop answering for it: the row that says
/// who answers is what orders its certificate.
pub async fn delete(
    database: &SqliteDatabase,
    service_id: &str,
    hostname: &str,
) -> PlatformResult<()> {
    let (service_id, hostname) = (service_id.to_string(), hostname.to_string());
    let removed = database
        .transaction(move |transaction| {
            let removed = transaction.execute(
                "DELETE FROM service_redirect WHERE \"hostname\" = ?1 AND \"port_id\" IN \
                   (SELECT \"id\" FROM port WHERE \"service_id\" = ?2)",
                (&hostname, &service_id),
            )?;
            if removed > 0 {
                transaction.execute(
                    "DELETE FROM service_edge WHERE \"hostname\" = ?1 AND \"service_id\" = ?2",
                    (&hostname, &service_id),
                )?;
            }
            Ok(removed)
        })
        .await?;
    match removed {
        0 => Err(PlatformError::Refused(NO_SUCH_REDIRECT.into())),
        _ => Ok(()),
    }
}

const COLUMNS: &str = "SELECT r.\"hostname\", r.\"port_id\", p.\"service_id\", p.\"hostname\" \
     FROM service_redirect r JOIN port p ON p.\"id\" = r.\"port_id\" \
     WHERE p.\"hostname\" IS NOT NULL";

fn read(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Redirect> {
    Ok(Redirect {
        hostname: row.get(0)?,
        port_id: row.get(1)?,
        service_id: row.get(2)?,
        target: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::repositories::{edges, ports};

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "site")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "web",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        (database, service.id)
    }

    async fn named(database: &SqliteDatabase, service: &str, hostname: Option<&str>) -> Port {
        ports::create(database, service, 80, false, hostname, &["nd-edge".into()])
            .await
            .expect("port")
    }

    fn refused<T: std::fmt::Debug>(result: PlatformResult<T>, reason: &str) {
        assert!(
            matches!(&result, Err(PlatformError::Refused(said)) if said == reason),
            "expected {reason:?}, got {result:?}"
        );
    }

    /// The target is read off the port, so the redirect follows a renamed
    /// port, and its own name is answered for like any other — which is
    /// what orders its certificate.
    #[tokio::test]
    async fn a_redirect_sends_its_name_to_whatever_its_port_answers_to() {
        let (database, service) = service().await;
        let port = named(&database, &service, Some("teresaasprilla.com")).await;
        create(
            &database,
            &port,
            "WWW.TeresaAsprilla.com.",
            &["nd-edge".into()],
        )
        .await
        .expect("redirect");

        let made = of_service(&database, &service).await.expect("redirects");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].hostname, "www.teresaasprilla.com");
        assert_eq!(made[0].target, "teresaasprilla.com");
        assert_eq!(
            edges::nodes_for(&database, "www.teresaasprilla.com")
                .await
                .expect("edges"),
            vec!["nd-edge"]
        );

        ports::set_hostname(&database, &port.id, Some("teresa.example"))
            .await
            .expect("renamed");
        assert_eq!(
            all(&database).await.expect("redirects")[0].target,
            "teresa.example"
        );
    }

    #[tokio::test]
    async fn a_name_either_serves_or_redirects() {
        let (database, service) = service().await;
        let port = named(&database, &service, Some("teresaasprilla.com")).await;
        let unnamed = ports::create(&database, &service, 8080, false, None, &[])
            .await
            .expect("a port with no name");

        refused(
            create(&database, &port, "teresaasprilla.com", &[]).await,
            TO_ITSELF,
        );
        refused(
            create(&database, &unnamed, "www.teresaasprilla.com", &[]).await,
            NO_TARGET,
        );
        let other = ports::create(
            &database,
            &service,
            81,
            false,
            Some("cloud.teresaasprilla.com"),
            &[],
        )
        .await
        .expect("a second named port");
        refused(
            create(&database, &port, "cloud.teresaasprilla.com", &[]).await,
            IS_A_HOSTNAME,
        );
        create(&database, &port, "www.teresaasprilla.com", &[])
            .await
            .expect("redirect");
        refused(
            create(&database, &other, "www.teresaasprilla.com", &[]).await,
            ALREADY_REDIRECTS,
        );
        refused(
            ports::create(
                &database,
                &service,
                9000,
                false,
                Some("www.teresaasprilla.com"),
                &[],
            )
            .await,
            ports::REDIRECTS_ALREADY,
        );
        refused(
            ports::set_hostname(&database, &other.id, Some("www.teresaasprilla.com")).await,
            ports::REDIRECTS_ALREADY,
        );
    }

    /// The redirect row would go with its port by itself. The edge row is
    /// keyed on the name and would not — and left behind it is a
    /// certificate ordered twice a day for a name nothing answers.
    #[tokio::test]
    async fn a_redirect_goes_with_its_port_and_its_name_with_it() {
        let (database, service) = service().await;
        let port = named(&database, &service, Some("teresaasprilla.com")).await;
        create(
            &database,
            &port,
            "www.teresaasprilla.com",
            &["nd-edge".into()],
        )
        .await
        .expect("redirect");

        ports::delete(&database, &port.id).await.expect("deleted");
        assert!(all(&database).await.expect("redirects").is_empty());
        assert!(edges::nodes_for(&database, "www.teresaasprilla.com")
            .await
            .expect("edges")
            .is_empty());

        let port = named(&database, &service, Some("teresaasprilla.com")).await;
        create(
            &database,
            &port,
            "www.teresaasprilla.com",
            &["nd-edge".into()],
        )
        .await
        .expect("again");
        ports::set_hostname(&database, &port.id, None)
            .await
            .expect("unnamed");
        assert!(all(&database).await.expect("redirects").is_empty());
        assert!(edges::nodes_for(&database, "www.teresaasprilla.com")
            .await
            .expect("edges")
            .is_empty());
    }

    #[tokio::test]
    async fn deleting_a_redirect_stops_answering_for_its_name() {
        let (database, service) = service().await;
        let port = named(&database, &service, Some("teresaasprilla.com")).await;
        create(
            &database,
            &port,
            "www.teresaasprilla.com",
            &["nd-edge".into()],
        )
        .await
        .expect("redirect");

        refused(
            delete(&database, "svc-somebody-else", "www.teresaasprilla.com").await,
            NO_SUCH_REDIRECT,
        );
        delete(&database, &service, "www.teresaasprilla.com")
            .await
            .expect("deleted");
        assert!(all(&database).await.expect("redirects").is_empty());
        assert!(edges::nodes_for(&database, "www.teresaasprilla.com")
            .await
            .expect("edges")
            .is_empty());
        assert_eq!(
            edges::nodes_for(&database, "teresaasprilla.com")
                .await
                .expect("edges"),
            vec!["nd-edge"],
            "the target's own name is untouched"
        );
    }
}
