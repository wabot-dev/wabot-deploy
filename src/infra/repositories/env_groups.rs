use std::collections::BTreeMap;

use wabot::sqlite::rusqlite::Row;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::env_group::EnvGroup;
use crate::domain::entities::service::Service;
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::infra::clock::now_ms;

pub const ALREADY_NAMED: &str = "this project already has a group by that name";

pub async fn of_project(
    database: &SqliteDatabase,
    project_id: &str,
) -> PlatformResult<Vec<EnvGroup>> {
    let project_id = project_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "{COLUMNS} WHERE \"project_id\" = ?1 ORDER BY \"name\""
                ))?
                .query_map([project_id], read)?
                .collect()
        })
        .await?)
}

pub async fn find(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
) -> PlatformResult<Option<EnvGroup>> {
    let (project_id, name) = (project_id.to_string(), name.to_string());
    Ok(database
        .read(move |connection| {
            let mut found = connection
                .prepare(&format!(
                    "{COLUMNS} WHERE \"project_id\" = ?1 AND \"name\" = ?2"
                ))?
                .query_map((project_id, name), read)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(found.pop())
        })
        .await?)
}

pub async fn create(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
    env: &BTreeMap<String, String>,
) -> PlatformResult<EnvGroup> {
    let group = EnvGroup {
        id: format!("env-{}", wabot::prelude::password::generate(12)),
        project_id: project_id.to_string(),
        name: name.to_string(),
        env: env.clone(),
    };
    let (id, project, name, payload) = (
        group.id.clone(),
        group.project_id.clone(),
        group.name.clone(),
        serde_json::to_string(env).unwrap_or_else(|_| "{}".into()),
    );
    database
        .write(move |connection| {
            let now = now_ms();
            connection.execute(
                "INSERT INTO env_group (\"id\", \"project_id\", \"name\", \"env\", \"created_at\", \"updated_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                (id, project, name, payload, now),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| match error.to_string().contains("UNIQUE") {
            true => PlatformError::Refused(ALREADY_NAMED.into()),
            false => PlatformError::Storage(error),
        })?;
    Ok(group)
}

pub async fn set_env(
    database: &SqliteDatabase,
    group_id: &str,
    env: &BTreeMap<String, String>,
) -> PlatformResult<()> {
    let id = group_id.to_string();
    let payload = serde_json::to_string(env).unwrap_or_else(|_| "{}".into());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE env_group SET \"env\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
                (id, payload, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn delete(database: &SqliteDatabase, group_id: &str) -> PlatformResult<()> {
    let id = group_id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM env_group WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<EnvGroup>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "{COLUMNS} WHERE \"id\" IN \
                       (SELECT \"group_id\" FROM service_env_group WHERE \"service_id\" = ?1) \
                     ORDER BY \"name\""
                ))?
                .query_map([service_id], read)?
                .collect()
        })
        .await?)
}

pub async fn set_for_service(
    database: &SqliteDatabase,
    service_id: &str,
    group_ids: &[String],
) -> PlatformResult<()> {
    let (service_id, group_ids) = (service_id.to_string(), group_ids.to_vec());
    database
        .transaction(move |transaction| {
            transaction.execute(
                "DELETE FROM service_env_group WHERE \"service_id\" = ?1",
                [&service_id],
            )?;
            for group_id in group_ids {
                transaction.execute(
                    "INSERT OR IGNORE INTO service_env_group (\"service_id\", \"group_id\") VALUES (?1, ?2)",
                    (&service_id, group_id),
                )?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn services_using(
    database: &SqliteDatabase,
    group_id: &str,
) -> PlatformResult<Vec<String>> {
    let group_id = group_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"service_id\" FROM service_env_group WHERE \"group_id\" = ?1 \
                     ORDER BY \"service_id\"",
                )?
                .query_map([group_id], |row| row.get(0))?
                .collect()
        })
        .await?)
}

/// Read at every deployment rather than copied into the service, so a
/// group saved once is what every one of its services starts with next.
pub async fn environment(
    database: &SqliteDatabase,
    service: &Service,
) -> PlatformResult<BTreeMap<String, String>> {
    let groups = of_service(database, &service.id).await?;
    Ok(crate::domain::entities::env_group::environment(
        &groups,
        &service.env,
    ))
}

const COLUMNS: &str = "SELECT \"id\", \"project_id\", \"name\", \"env\" FROM env_group";

fn read(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<EnvGroup> {
    let env: String = row.get(3)?;
    Ok(EnvGroup {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        env: serde_json::from_str(&env).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn world() -> (SqliteDatabase, Service, Service) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");
        let made = |name: &'static str| {
            let (database, project) = (database.clone(), project.id.clone());
            async move {
                crate::infra::repositories::services::create(
                    &database,
                    &project,
                    name,
                    "docker.io/library/nginx:alpine",
                    &[("PORT".to_string(), "8080".to_string())],
                )
                .await
                .expect("service")
            }
        };
        let (orders, ledger) = (made("orders").await, made("ledger").await);
        (database, orders, ledger)
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn a_group_saved_once_reaches_every_service_that_joined_it() {
        let (database, orders, ledger) = world().await;
        let smtp = create(
            &database,
            &orders.project_id,
            "smtp",
            &env(&[("SMTP_HOST", "mail")]),
        )
        .await
        .expect("group");
        for service in [&orders, &ledger] {
            set_for_service(&database, &service.id, std::slice::from_ref(&smtp.id))
                .await
                .expect("joined");
        }

        set_env(&database, &smtp.id, &env(&[("SMTP_HOST", "mail.eu")]))
            .await
            .expect("saved");

        for service in [&orders, &ledger] {
            let got = environment(&database, service).await.expect("read");
            assert_eq!(got.get("SMTP_HOST").map(String::as_str), Some("mail.eu"));
            assert_eq!(got.get("PORT").map(String::as_str), Some("8080"));
        }
        assert_eq!(
            services_using(&database, &smtp.id)
                .await
                .expect("read")
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn a_name_is_one_group_per_project() {
        let (database, orders, _) = world().await;
        create(&database, &orders.project_id, "smtp", &BTreeMap::new())
            .await
            .expect("first");
        let again = create(&database, &orders.project_id, "smtp", &BTreeMap::new()).await;
        assert!(
            matches!(&again, Err(PlatformError::Refused(reason)) if reason == ALREADY_NAMED),
            "{again:?}"
        );
    }

    /// A project takes its services and its groups by cascade, which no
    /// caller sees.
    #[tokio::test]
    async fn a_group_goes_with_its_project() {
        let (database, orders, _) = world().await;
        let smtp = create(&database, &orders.project_id, "smtp", &BTreeMap::new())
            .await
            .expect("group");
        set_for_service(&database, &orders.id, std::slice::from_ref(&smtp.id))
            .await
            .expect("joined");

        crate::infra::repositories::projects::delete(&database, &orders.project_id)
            .await
            .expect("delete");

        assert!(of_project(&database, &orders.project_id)
            .await
            .expect("read")
            .is_empty());
        assert!(services_using(&database, &smtp.id)
            .await
            .expect("read")
            .is_empty());
    }
}
