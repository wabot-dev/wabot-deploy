//! `wabot-deploy import` — what a service had somewhere else, brought here:
//! a dump into a managed database that holds nothing yet, or an archive into
//! one of a service's volumes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::infra::config::Config;

pub async fn import(
    config: Config,
    service: String,
    from: PathBuf,
    volume: Option<String>,
    owner: Option<String>,
) -> anyhow::Result<i32> {
    let rows = crate::infra::db::open(&config.database_path()).await?;
    let outcome = match volume {
        Some(volume) => fill(&config, &rows, &service, &volume, &from, owner.as_deref()).await,
        None => load(&config, &rows, &service, &from).await,
    };
    rows.close().await?;
    match outcome {
        Ok(said) => {
            println!("{said}");
            Ok(0)
        }
        Err(reason) => {
            println!("{reason}");
            Ok(1)
        }
    }
}

async fn load(
    config: &Config,
    rows: &wabot::sqlite::SqliteDatabase,
    named: &str,
    from: &Path,
) -> Result<String, String> {
    let (project, service) = find(rows, named, true).await?;
    println!(
        "loading {} into {}/{}…",
        from.display(),
        project.slug,
        service.slug
    );
    let tables = crate::infra::deploy::Deployer::new(Arc::new(rows.clone()), config)
        .import_dump(&project, &service, from)
        .await?;
    Ok(format!(
        "loaded: {}/{} holds {tables} table(s)",
        project.slug, service.slug
    ))
}

async fn fill(
    config: &Config,
    rows: &wabot::sqlite::SqliteDatabase,
    named: &str,
    volume: &str,
    from: &Path,
    owner: Option<&str>,
) -> Result<String, String> {
    let owner = owner.map(parse_owner).transpose()?;
    let (project, service) = find(rows, named, false).await?;
    let kept = crate::infra::repositories::volumes::of_service(rows, &service.id)
        .await
        .map_err(|error| error.to_string())?;
    let Some(volume) = kept.iter().find(|kept| kept.name == volume) else {
        return Err(match kept.is_empty() {
            true => format!(
                "{}/{} keeps no volume — add one on its Storage tab first",
                project.slug, service.slug
            ),
            false => format!(
                "{}/{} keeps no volume called {volume:?}; it keeps {}",
                project.slug,
                service.slug,
                kept.iter()
                    .map(|kept| format!("{:?} at {}", kept.name, kept.path))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    };
    let copies: Vec<_> = crate::infra::repositories::replicas::of_service(rows, &service.id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|copy| copy.is_here() && !copy.evicted())
        .collect();
    if copies.is_empty() {
        return Err(format!(
            "no copy of {}/{} runs on this node, and a volume is a directory on the node that runs it",
            project.slug, service.slug
        ));
    }

    for copy in &copies {
        let id = copy.container_id(&project.slug, &service.slug);
        println!("unpacking {} into {id} at {}…", from.display(), volume.path);
        let filled = crate::infra::repositories::volumes::fill(
            &config.node.data_dir,
            &id,
            &volume.name,
            from,
            owner,
        )
        .map_err(|error| format!("{}: {error}", from.display()))?;
        println!("  {} entries", filled.entries);
        for skipped in &filled.skipped {
            println!("  skipped {skipped}: it would have been written outside the volume");
        }
    }
    Ok(format!(
        "filled: deploy {}/{} again for it to read what it was given",
        project.slug, service.slug
    ))
}

/// `UID` or `UID:GID`; the group is the user's when left out.
fn parse_owner(text: &str) -> Result<(u32, u32), String> {
    let refused = || format!("{text:?} is not an owner: give UID or UID:GID, as numbers");
    let (uid, gid) = match text.split_once(':') {
        Some((uid, gid)) => (uid, gid),
        None => (text, text),
    };
    Ok((
        uid.trim().parse().map_err(|_| refused())?,
        gid.trim().parse().map_err(|_| refused())?,
    ))
}

/// A service by the name the console shows, or by `<project>/<name>` when
/// that is not enough to say which. `database` says which kind is meant: a
/// dump goes into a database, files into a service's volume.
async fn find(
    rows: &wabot::sqlite::SqliteDatabase,
    named: &str,
    database: bool,
) -> Result<(Project, Service), String> {
    let (in_project, slug) = match named.split_once('/') {
        Some((project, slug)) => (Some(project), slug),
        None => (None, named),
    };
    let projects = crate::infra::repositories::projects::all(rows)
        .await
        .map_err(|error| error.to_string())?;
    let mut found: Vec<(Project, Service)> = crate::infra::repositories::services::all(rows, None)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|service| service.kind.is_managed() == database && service.slug == slug)
        .filter_map(|service| {
            let project = projects
                .iter()
                .find(|project| project.id == service.project_id)?;
            Some((project.clone(), service))
        })
        .filter(|(project, _)| in_project.is_none_or(|wanted| project.slug == wanted))
        .collect();
    match found.len() {
        0 => Err(match database {
            true => format!("no database called {named:?} on this node"),
            false => format!(
                "no service called {named:?} on this node — a database is loaded from a dump, \
                 without --volume"
            ),
        }),
        1 => Ok(found.remove(0)),
        _ => Err(format!(
            "{named:?} is a database in more than one project — name it as one of: {}",
            found
                .iter()
                .map(|(project, service)| format!("{}/{}", project.slug, service.slug))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database_in(rows: &wabot::sqlite::SqliteDatabase, project: &str) {
        let made = crate::infra::repositories::projects::create(rows, project)
            .await
            .expect("project");
        crate::features::databases::create_database_command::CreateDatabase {
            project_id: &made.id,
            name: "orders",
            engine: crate::domain::entities::database::Engine::Mysql,
            version: "8.4",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(rows)
        .await
        .expect("database");
    }

    #[tokio::test]
    async fn a_name_two_projects_share_is_asked_for_by_project() {
        let rows = crate::infra::db::open_in_memory().await.expect("open");
        database_in(&rows, "shop").await;
        database_in(&rows, "blog").await;

        let refused = find(&rows, "orders", true).await.expect_err("ambiguous");
        assert!(
            refused.contains("shop/orders") && refused.contains("blog/orders"),
            "{refused}"
        );

        let (project, service) = find(&rows, "blog/orders", true).await.expect("named");
        assert_eq!(
            (project.slug.as_str(), service.slug.as_str()),
            ("blog", "orders")
        );
        assert!(find(&rows, "elsewhere/orders", true).await.is_err());
        assert!(
            find(&rows, "blog/orders", false).await.is_err(),
            "a database's files are not filled from an archive"
        );
    }
}

#[cfg(test)]
mod owner_tests {
    use super::parse_owner;

    #[test]
    fn an_owner_is_a_user_and_a_group_and_the_group_defaults_to_the_user() {
        assert_eq!(parse_owner("1000"), Ok((1000, 1000)));
        assert_eq!(parse_owner("1000:50"), Ok((1000, 50)));
        assert!(parse_owner("node").is_err());
        assert!(parse_owner("1000:").is_err());
    }
}
