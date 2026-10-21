//! A DNS zone's rows: the zone, its records, and which nodes answer for it.

use wabot::sqlite::rusqlite::Row;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::dns_record::DnsRecord;
use crate::domain::entities::dns_zone::{Delegation, DelegationStatus, DnsZone, DEFAULT_TTL};
use crate::domain::entities::project::Project;
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::policies::zone_serial;
use crate::domain::values::record_data::{RecordData, RecordKind};
use crate::domain::values::record_name::RecordName;
use crate::domain::values::zone_name::ZoneName;
use crate::infra::clock::now_ms;

pub const ALREADY_SERVED: &str = "this node already has a zone for that domain";
pub const OVERLAPS: &str =
    "that domain overlaps a zone outside this project — a name has one owner";
pub const PROJECT_SENT_HERE: &str =
    "that project is administered from the node that sent it, and its zones are made there";
pub const HOLDS_THE_NODE: &str =
    "that domain holds this node's own name, which is the node operator's to manage";
pub const HOLDS_AN_ORGANISATION: &str =
    "that domain holds the name another organisation's console answers to";
pub const CNAME_STANDS_ALONE: &str = "a CNAME stands alone: that name already has other records, \
     and a name with a CNAME can have nothing else";
pub const ALREADY_THERE: &str = "that record is already there";
pub const NO_SUCH_ZONE: &str = "there is no such zone";
pub const NO_SUCH_RECORD: &str = "there is no such record";
pub const OURS_ALREADY: &str = "this node has a zone of its own for that domain";
pub const HELD_FOR_ANOTHER: &str = "that domain is answered here for another node already";

/// What creating a zone needs to know: the project it goes into, which is
/// whose it is, and nothing about whether the asker may.
pub struct NewZone<'a> {
    pub name: &'a ZoneName,
    pub project: &'a Project,
    pub servers: &'a [String],
}

/// **A name has one owner, and the owner is a project.** Two projects of
/// one organisation are still two sets of people, and a zone nested in
/// another's would answer for names the other's people wrote.
pub async fn create(database: &SqliteDatabase, asked: NewZone<'_>) -> PlatformResult<DnsZone> {
    if !asked.project.is_ours() {
        return Err(PlatformError::Refused(PROJECT_SENT_HERE.into()));
    }
    let zones = all(database).await?;
    if zones.iter().any(|zone| zone.name == *asked.name) {
        return Err(PlatformError::Refused(ALREADY_SERVED.into()));
    }
    let overlapping = zones.iter().any(|zone| {
        let same_owner = zone.project_id.as_deref() == Some(asked.project.id.as_str());
        !same_owner
            && (zone.name.holds(asked.name.as_str()) || asked.name.holds(zone.name.as_str()))
    });
    if overlapping {
        return Err(PlatformError::Refused(OVERLAPS.into()));
    }
    if let Some(tenant) = asked.project.tenant_id.as_deref() {
        if let Some(domain) = crate::infra::node::settings::domain(database).await {
            if asked.name.holds(&domain) {
                return Err(PlatformError::Refused(HOLDS_THE_NODE.into()));
            }
        }
        for (other, domain) in tenant_domains(database).await? {
            if other != tenant && asked.name.holds(&domain) {
                return Err(PlatformError::Refused(HOLDS_AN_ORGANISATION.into()));
            }
        }
    }

    let now = now_ms();
    let zone = DnsZone {
        id: format!("zone-{}", wabot::prelude::password::generate(12)),
        name: asked.name.clone(),
        project_id: Some(asked.project.id.clone()),
        authority_id: None,
        serial: zone_serial::next(0, (now / 1000) as u64),
        default_ttl: DEFAULT_TTL,
        delegation: None,
        received_name_servers: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    let row = zone.clone();
    let servers = asked.servers.to_vec();
    database
        .transaction(move |transaction| {
            transaction.execute(
                "INSERT INTO dns_zone (\"id\", \"name\", \"project_id\", \"authority_id\", \"serial\", \
                   \"default_ttl\", \"created_at\", \"updated_at\") \
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?6)",
                (
                    &row.id,
                    row.name.as_str(),
                    &row.project_id,
                    row.serial,
                    row.default_ttl,
                    row.created_at,
                ),
            )?;
            write_servers(transaction, &row.id, &servers)?;
            Ok(())
        })
        .await
        .map_err(|error| match error.to_string().contains("UNIQUE") {
            true => PlatformError::Refused(ALREADY_SERVED.into()),
            false => PlatformError::Storage(error),
        })?;
    Ok(zone)
}

pub async fn all(database: &SqliteDatabase) -> PlatformResult<Vec<DnsZone>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(&format!("{ZONE_COLUMNS} ORDER BY \"name\""))?
                .query_map([], read_zone)?
                .collect::<Result<Vec<_>, _>>()
        })
        .await?
        .into_iter()
        .flatten()
        .collect())
}

pub async fn of_project(
    database: &SqliteDatabase,
    project_id: &str,
) -> PlatformResult<Vec<DnsZone>> {
    let project_id = project_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "{ZONE_COLUMNS} WHERE \"project_id\" = ?1 ORDER BY \"name\""
                ))?
                .query_map([project_id], read_zone)?
                .collect::<Result<Vec<_>, _>>()
        })
        .await?
        .into_iter()
        .flatten()
        .collect())
}

/// By the domain, which is how the console addresses a zone: the name is
/// unique on a node, and it is what the breadcrumb and the address bar say.
pub async fn find_by_name(
    database: &SqliteDatabase,
    name: &str,
) -> PlatformResult<Option<DnsZone>> {
    let Ok(name) = ZoneName::parse(name) else {
        return Ok(None);
    };
    Ok(all(database)
        .await?
        .into_iter()
        .find(|zone| zone.name == name))
}

pub async fn records(database: &SqliteDatabase, zone: &DnsZone) -> PlatformResult<Vec<DnsRecord>> {
    let id = zone.id.clone();
    let rows: Vec<(String, String, String, String, Option<u16>, u32)> = database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"name\", \"kind\", \"value\", \"priority\", \"ttl\" \
                     FROM dns_record WHERE \"zone_id\" = ?1 ORDER BY \"name\", \"kind\", \"value\"",
                )?
                .query_map([id], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })?
                .collect()
        })
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, name, kind, value, priority, ttl)| {
            let parsed = RecordName::parse(&name, &zone.name).and_then(|name| {
                let kind = RecordKind::parse(&kind)?;
                let priority = priority.map(|priority| priority.to_string());
                let data = RecordData::parse(kind, &value, priority.as_deref())?;
                DnsRecord::new(id.clone(), zone.id.clone(), name, data, ttl)
            });
            match parsed {
                Ok(record) => Some(record),
                Err(reason) => {
                    tracing::warn!(record = %id, %reason, "a stored record that does not read back");
                    None
                }
            }
        })
        .collect())
}

pub async fn servers(database: &SqliteDatabase, zone_id: &str) -> PlatformResult<Vec<String>> {
    let id = zone_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"node_id\" FROM dns_zone_server WHERE \"zone_id\" = ?1 \
                     ORDER BY \"position\"",
                )?
                .query_map([id], |row| row.get(0))?
                .collect()
        })
        .await?)
}

pub async fn add_record(
    database: &SqliteDatabase,
    zone: &DnsZone,
    name: RecordName,
    data: RecordData,
    ttl: u32,
) -> PlatformResult<DnsRecord> {
    let record = DnsRecord::new(
        format!("rec-{}", wabot::prelude::password::generate(12)),
        zone.id.clone(),
        name,
        data,
        ttl,
    )
    .map_err(|reason| PlatformError::Refused(reason.into()))?;

    let existing = records(database, zone).await?;
    let at_name: Vec<&DnsRecord> = existing
        .iter()
        .filter(|other| other.name == record.name)
        .collect();
    let adding_cname = record.data.kind() == RecordKind::Cname;
    let name_has_cname = at_name
        .iter()
        .any(|other| other.data.kind() == RecordKind::Cname);
    if (adding_cname && !at_name.is_empty()) || (!adding_cname && name_has_cname) {
        return Err(PlatformError::Refused(CNAME_STANDS_ALONE.into()));
    }
    if at_name.iter().any(|other| other.data == record.data) {
        return Err(PlatformError::Refused(ALREADY_THERE.into()));
    }

    let row = record.clone();
    let zone_id = zone.id.clone();
    database
        .transaction(move |transaction| {
            transaction.execute(
                "INSERT INTO dns_record (\"id\", \"zone_id\", \"name\", \"kind\", \"value\", \
                   \"priority\", \"ttl\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                (
                    &row.id,
                    &row.zone_id,
                    row.name.as_str(),
                    row.data.kind().name(),
                    row.data.value(),
                    row.data.priority(),
                    row.ttl,
                    now_ms(),
                ),
            )?;
            bump(transaction, &zone_id)?;
            Ok(())
        })
        .await?;
    Ok(record)
}

pub async fn remove_record(
    database: &SqliteDatabase,
    zone: &DnsZone,
    record_id: &str,
) -> PlatformResult<()> {
    let (zone_id, record_id) = (zone.id.clone(), record_id.to_string());
    let removed = database
        .transaction(move |transaction| {
            let removed = transaction.execute(
                "DELETE FROM dns_record WHERE \"id\" = ?1 AND \"zone_id\" = ?2",
                (&record_id, &zone_id),
            )?;
            if removed > 0 {
                bump(transaction, &zone_id)?;
            }
            Ok(removed)
        })
        .await?;
    match removed {
        0 => Err(PlatformError::Refused(NO_SUCH_RECORD.into())),
        _ => Ok(()),
    }
}

pub async fn set_servers(
    database: &SqliteDatabase,
    zone: &DnsZone,
    servers: &[String],
) -> PlatformResult<()> {
    let (zone_id, servers) = (zone.id.clone(), servers.to_vec());
    database
        .transaction(move |transaction| {
            write_servers(transaction, &zone_id, &servers)?;
            bump(transaction, &zone_id)?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// A zone another node administers, as it arrived.
pub struct Received<'a> {
    pub name: &'a ZoneName,
    pub authority: &'a str,
    pub serial: u32,
    pub default_ttl: u32,
    pub records: Vec<(RecordName, RecordData, u32)>,
    pub name_servers: &'a [String],
}

/// Hold a zone for the node that sent it, replacing whatever it sent before.
///
/// **A domain has one authority here.** A second node asking for a zone
/// this one already holds for somebody is refused rather than merged — two
/// owners' records for one domain is not a conflict anything can settle —
/// and so is one this node administers itself.
pub async fn replace_received(
    database: &SqliteDatabase,
    received: Received<'_>,
) -> PlatformResult<()> {
    let existing = find_by_name(database, received.name.as_str()).await?;
    if let Some(zone) = &existing {
        if zone.is_ours() {
            return Err(PlatformError::Refused(OURS_ALREADY.into()));
        }
        if zone.authority_id.as_deref() != Some(received.authority) {
            return Err(PlatformError::Refused(HELD_FOR_ANOTHER.into()));
        }
    }

    let now = now_ms();
    let id = existing
        .map(|zone| zone.id)
        .unwrap_or_else(|| format!("zone-{}", wabot::prelude::password::generate(12)));
    let name = received.name.to_string();
    let authority = received.authority.to_string();
    let (serial, ttl) = (received.serial, received.default_ttl);
    let name_servers = received.name_servers.join(",");
    let records: Vec<Written> = received
        .records
        .into_iter()
        .map(|(name, data, ttl)| Written {
            id: format!("rec-{}", wabot::prelude::password::generate(12)),
            name: name.to_string(),
            kind: data.kind().name(),
            value: data.value(),
            priority: data.priority(),
            ttl,
        })
        .collect();

    database
        .transaction(move |transaction| {
            transaction.execute(
                "INSERT INTO dns_zone (\"id\", \"name\", \"project_id\", \"authority_id\", \"serial\", \
                   \"default_ttl\", \"name_servers\", \"created_at\", \"updated_at\") \
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?7) \
                 ON CONFLICT (\"id\") DO UPDATE SET \"serial\" = ?4, \"default_ttl\" = ?5, \
                   \"name_servers\" = ?6, \"updated_at\" = ?7",
                (&id, &name, &authority, serial, ttl, &name_servers, now),
            )?;
            transaction.execute("DELETE FROM dns_record WHERE \"zone_id\" = ?1", [&id])?;
            for record in &records {
                transaction.execute(
                    "INSERT INTO dns_record (\"id\", \"zone_id\", \"name\", \"kind\", \"value\", \
                       \"priority\", \"ttl\", \"created_at\") \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    (
                        &record.id,
                        &id,
                        &record.name,
                        record.kind,
                        &record.value,
                        record.priority,
                        record.ttl,
                        now,
                    ),
                )?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

struct Written {
    id: String,
    name: String,
    kind: &'static str,
    value: String,
    priority: Option<u16>,
    ttl: u32,
}

/// Stop holding a zone for the node that sent it. Whether it was there is
/// the answer, so withdrawing twice is not an error.
pub async fn forget_received(
    database: &SqliteDatabase,
    name: &ZoneName,
    authority: &str,
) -> PlatformResult<bool> {
    match find_by_name(database, name.as_str()).await? {
        Some(zone) if zone.authority_id.as_deref() == Some(authority) => {
            delete(database, &zone).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub async fn delete(database: &SqliteDatabase, zone: &DnsZone) -> PlatformResult<()> {
    let id = zone.id.clone();
    database
        .transaction(move |transaction| {
            transaction.execute("DELETE FROM dns_record WHERE \"zone_id\" = ?1", [&id])?;
            transaction.execute("DELETE FROM dns_zone_server WHERE \"zone_id\" = ?1", [&id])?;
            transaction.execute("DELETE FROM dns_zone WHERE \"id\" = ?1", [&id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The note holds the parent's list, or the reason there is none — one
/// column, because a check produces one or the other.
pub async fn save_delegation(
    database: &SqliteDatabase,
    zone: &DnsZone,
    delegation: &Delegation,
) -> PlatformResult<()> {
    let note = match &delegation.reason {
        Some(reason) => reason.clone(),
        None => delegation.delegated.join(","),
    };
    let (id, status, at) = (
        zone.id.clone(),
        delegation.status.as_str(),
        delegation.checked_at,
    );
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE dns_zone SET \"delegation_status\" = ?2, \"delegation_note\" = ?3, \
                   \"delegation_checked_at\" = ?4 WHERE \"id\" = ?1",
                (id, status, note, at),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

const ZONE_COLUMNS: &str =
    "SELECT \"id\", \"name\", \"project_id\", \"authority_id\", \"serial\", \
     \"default_ttl\", \"delegation_status\", \"delegation_note\", \"delegation_checked_at\", \
     \"created_at\", \"updated_at\", \"name_servers\" FROM dns_zone";

/// `None` for a row whose name no longer reads as a zone — written by a
/// newer version, or by hand — so one bad row cannot take every zone off
/// the air with it.
fn read_zone(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Option<DnsZone>> {
    let name: String = row.get(1)?;
    let Ok(name) = ZoneName::parse(&name) else {
        tracing::warn!(zone = %name, "a stored zone whose name does not read back");
        return Ok(None);
    };
    let delegation = match (
        row.get::<_, Option<String>>(6)?,
        row.get::<_, Option<String>>(7)?,
        row.get::<_, Option<i64>>(8)?,
    ) {
        (Some(status), note, Some(checked_at)) => {
            let status = DelegationStatus::parse(&status);
            let note = note.unwrap_or_default();
            let (delegated, reason) = match status {
                DelegationStatus::Unknown => (Vec::new(), Some(note)),
                _ => (
                    note.split(',')
                        .filter(|name| !name.is_empty())
                        .map(str::to_string)
                        .collect(),
                    None,
                ),
            };
            Some(Delegation {
                status,
                delegated,
                reason,
                checked_at,
            })
        }
        _ => None,
    };
    Ok(Some(DnsZone {
        id: row.get(0)?,
        name,
        project_id: row.get(2)?,
        authority_id: row.get(3)?,
        serial: row.get(4)?,
        default_ttl: row.get(5)?,
        delegation,
        received_name_servers: row
            .get::<_, Option<String>>(11)?
            .map(|names| {
                names
                    .split(',')
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    }))
}

fn write_servers(
    transaction: &wabot::sqlite::rusqlite::Transaction<'_>,
    zone_id: &str,
    servers: &[String],
) -> wabot::sqlite::rusqlite::Result<()> {
    transaction.execute(
        "DELETE FROM dns_zone_server WHERE \"zone_id\" = ?1",
        [zone_id],
    )?;
    for (position, node) in servers.iter().enumerate() {
        transaction.execute(
            "INSERT OR IGNORE INTO dns_zone_server (\"zone_id\", \"node_id\", \"position\") \
             VALUES (?1, ?2, ?3)",
            (zone_id, node, position as i64),
        )?;
    }
    Ok(())
}

fn bump(
    transaction: &wabot::sqlite::rusqlite::Transaction<'_>,
    zone_id: &str,
) -> wabot::sqlite::rusqlite::Result<()> {
    let previous: u32 = transaction.query_row(
        "SELECT \"serial\" FROM dns_zone WHERE \"id\" = ?1",
        [zone_id],
        |row| row.get(0),
    )?;
    let now = now_ms();
    transaction.execute(
        "UPDATE dns_zone SET \"serial\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
        (
            zone_id,
            zone_serial::next(previous, (now / 1000) as u64),
            now,
        ),
    )?;
    Ok(())
}

async fn tenant_domains(database: &SqliteDatabase) -> PlatformResult<Vec<(String, String)>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare("SELECT \"id\", \"domain\" FROM tenant WHERE \"domain\" IS NOT NULL")?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::infra::db::open_in_memory().await.expect("open")
    }

    fn received<'a>(
        name: &'a ZoneName,
        authority: &'a str,
        serial: u32,
        servers: &'a [String],
    ) -> Received<'a> {
        Received {
            name,
            authority,
            serial,
            default_ttl: 300,
            records: vec![(
                RecordName::apex(),
                RecordData::parse(RecordKind::A, "209.50.48.176", None).expect("data"),
                300,
            )],
            name_servers: servers,
        }
    }

    /// The copy a node holds for another is the last one it was sent,
    /// whole: nothing of an earlier one survives, and nobody else's arrives.
    #[tokio::test]
    async fn a_received_zone_is_the_last_one_sent_and_only_by_its_sender() {
        let database = database().await;
        let name = zone_name("teresaasprilla.com");
        let servers = vec![
            "node-0.example.net".to_string(),
            "node-1.example.net".to_string(),
        ];

        replace_received(&database, received(&name, "nd-owner", 10, &servers))
            .await
            .expect("first");
        let mut again = received(&name, "nd-owner", 11, &servers[..1]);
        again.records.clear();
        replace_received(&database, again).await.expect("again");

        let zone = find_by_name(&database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("there");
        assert_eq!(zone.authority_id.as_deref(), Some("nd-owner"));
        assert_eq!(zone.serial, 11);
        assert_eq!(zone.received_name_servers, vec!["node-0.example.net"]);
        assert!(records(&database, &zone).await.expect("records").is_empty());

        let other = replace_received(&database, received(&name, "nd-stranger", 12, &servers)).await;
        assert!(matches!(other, Err(PlatformError::Refused(reason)) if reason == HELD_FOR_ANOTHER));
        assert!(!forget_received(&database, &name, "nd-stranger")
            .await
            .expect("forget"));
        assert!(forget_received(&database, &name, "nd-owner")
            .await
            .expect("forget"));
        assert!(find_by_name(&database, "teresaasprilla.com")
            .await
            .expect("read")
            .is_none());
    }

    #[tokio::test]
    async fn a_node_does_not_take_a_zone_for_a_domain_it_administers_itself() {
        let database = database().await;
        let site = project(&database, "site", None).await;
        zone(&database, "teresaasprilla.com", &site)
            .await
            .expect("ours");
        let name = zone_name("teresaasprilla.com");
        let refused = replace_received(&database, received(&name, "nd-owner", 10, &[])).await;
        assert!(matches!(refused, Err(PlatformError::Refused(reason)) if reason == OURS_ALREADY));
    }

    fn zone_name(name: &str) -> ZoneName {
        ZoneName::parse(name).expect("zone")
    }

    async fn zone(
        database: &SqliteDatabase,
        name: &str,
        project: &Project,
    ) -> PlatformResult<DnsZone> {
        create(
            database,
            NewZone {
                name: &zone_name(name),
                project,
                servers: &["nd-self".to_string()],
            },
        )
        .await
    }

    async fn project(database: &SqliteDatabase, name: &str, tenant: Option<&str>) -> Project {
        crate::infra::repositories::projects::create_for(database, name, tenant)
            .await
            .expect("project")
    }

    async fn tenant(database: &SqliteDatabase, id: &str, domain: Option<&str>) {
        let (id, domain) = (id.to_string(), domain.map(str::to_string));
        database
            .write(move |connection| {
                connection.execute(
                    "INSERT INTO tenant (\"id\", \"name\", \"slug\", \"domain\", \"created_at\") \
                     VALUES (?1, ?1, ?1, ?2, 0)",
                    (id, domain),
                )?;
                Ok(())
            })
            .await
            .expect("tenant");
    }

    fn data(kind: RecordKind, value: &str) -> RecordData {
        RecordData::parse(kind, value, None).expect(value)
    }

    fn name(zone: &DnsZone, typed: &str) -> RecordName {
        RecordName::parse(typed, &zone.name).expect(typed)
    }

    #[tokio::test]
    async fn a_domain_has_one_zone_on_a_node() {
        let database = database().await;
        let site = project(&database, "site", None).await;
        let other = project(&database, "other", None).await;
        zone(&database, "teresaasprilla.com", &site)
            .await
            .expect("first");
        for project in [&site, &other] {
            let second = zone(&database, "TeresaAsprilla.com.", project).await;
            assert!(
                matches!(&second, Err(PlatformError::Refused(reason)) if reason == ALREADY_SERVED),
                "{second:?}"
            );
        }
    }

    /// A project that could add `cloud.teresaasprilla.com` under another's
    /// `teresaasprilla.com`, or the other way round, would be answering for
    /// names other people wrote — and that holds between two projects of one
    /// organisation, and two of the operator's, as much as between strangers.
    #[tokio::test]
    async fn one_project_cannot_reach_into_another_projects_zone() {
        let database = database().await;
        tenant(&database, "tnt-teresa", None).await;
        tenant(&database, "tnt-other", None).await;
        let hers = project(&database, "teresa", Some("tnt-teresa")).await;
        zone(&database, "teresaasprilla.com", &hers)
            .await
            .expect("hers");

        let strangers = [
            project(&database, "teresa blog", Some("tnt-teresa")).await,
            project(&database, "other", Some("tnt-other")).await,
            project(&database, "operator", None).await,
        ];
        for stranger in &strangers {
            let refused = zone(&database, "cloud.teresaasprilla.com", stranger).await;
            assert!(
                matches!(&refused, Err(PlatformError::Refused(reason)) if reason == OVERLAPS),
                "inside hers, from {}: {refused:?}",
                stranger.slug
            );
        }
        let refused = zone(&database, "asprilla.com", &strangers[1]).await;
        assert!(
            refused.is_ok(),
            "a different domain with similar letters: {refused:?}"
        );
        zone(&database, "blog.teresaasprilla.com", &hers)
            .await
            .expect("her own subdomain is hers");
    }

    #[tokio::test]
    async fn a_project_sent_from_another_node_makes_its_zones_there() {
        let database = database().await;
        let sent = project(&database, "sent", None).await;
        crate::infra::repositories::projects::set_origin(&database, &sent.id, "nd-far")
            .await
            .expect("origin");
        let sent = crate::infra::repositories::projects::find(&database, &sent.id)
            .await
            .expect("read")
            .expect("there");

        let refused = zone(&database, "teresaasprilla.com", &sent).await;
        assert!(
            matches!(&refused, Err(PlatformError::Refused(reason)) if reason == PROJECT_SENT_HERE),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn an_organisation_cannot_take_another_organisations_console_name() {
        let database = database().await;
        tenant(&database, "tnt-teresa", Some("cloud.teresaasprilla.com")).await;
        tenant(&database, "tnt-other", None).await;

        let other = project(&database, "other", Some("tnt-other")).await;
        let refused = zone(&database, "teresaasprilla.com", &other).await;
        assert!(
            matches!(refused, Err(PlatformError::Refused(reason)) if reason == HOLDS_AN_ORGANISATION)
        );
        let hers = project(&database, "teresa", Some("tnt-teresa")).await;
        zone(&database, "teresaasprilla.com", &hers)
            .await
            .expect("the organisation whose console it is may hold it");
    }

    #[tokio::test]
    async fn an_organisation_cannot_take_the_nodes_own_name() {
        let database = database().await;
        tenant(&database, "tnt-other", None).await;
        crate::infra::node::settings::set_domain(&database, Some("node-0.network.nubebasura.lat"))
            .await
            .expect("domain");

        let other = project(&database, "other", Some("tnt-other")).await;
        let refused = zone(&database, "nubebasura.lat", &other).await;
        assert!(matches!(refused, Err(PlatformError::Refused(reason)) if reason == HOLDS_THE_NODE));
        let operator = project(&database, "operator", None).await;
        zone(&database, "nubebasura.lat", &operator)
            .await
            .expect("the operator's own domain is the operator's");
    }

    #[tokio::test]
    async fn a_projects_zones_are_its_own_and_nobody_elses() {
        let database = database().await;
        let site = project(&database, "site", None).await;
        let other = project(&database, "other", None).await;
        zone(&database, "teresaasprilla.com", &site)
            .await
            .expect("site's");
        zone(&database, "asprilla.com", &other)
            .await
            .expect("other's");

        let listed: Vec<String> = of_project(&database, &site.id)
            .await
            .expect("site's zones")
            .into_iter()
            .map(|zone| zone.name.to_string())
            .collect();
        assert_eq!(listed, vec!["teresaasprilla.com"]);
    }

    /// The project's rows go with it, and the zone's with those — which is
    /// what the foreign keys promise and a schema change could quietly break.
    #[tokio::test]
    async fn deleting_a_project_takes_its_zones_and_their_records() {
        let database = database().await;
        let site = project(&database, "site", None).await;
        let created = zone(&database, "teresaasprilla.com", &site)
            .await
            .expect("zone");
        add_record(
            &database,
            &created,
            name(&created, "www"),
            data(RecordKind::A, "209.50.48.176"),
            300,
        )
        .await
        .expect("record");

        crate::infra::repositories::projects::delete(&database, &site.id)
            .await
            .expect("delete");
        assert!(all(&database).await.expect("zones").is_empty());
        let left: i64 = database
            .read(|connection| {
                connection.query_row(
                    "SELECT (SELECT COUNT(*) FROM dns_record) + (SELECT COUNT(*) FROM dns_zone_server)",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("count");
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn a_cname_stands_alone_at_its_name() {
        let database = database().await;
        let site = project(&database, "site", None).await;
        let zone = zone(&database, "teresaasprilla.com", &site)
            .await
            .expect("zone");

        add_record(
            &database,
            &zone,
            name(&zone, "www"),
            data(RecordKind::A, "209.50.48.176"),
            300,
        )
        .await
        .expect("an address");
        let refused = add_record(
            &database,
            &zone,
            name(&zone, "www"),
            data(RecordKind::Cname, "teresaasprilla.com"),
            300,
        )
        .await;
        assert!(
            matches!(refused, Err(PlatformError::Refused(reason)) if reason == CNAME_STANDS_ALONE)
        );

        add_record(
            &database,
            &zone,
            name(&zone, "email.mg"),
            data(RecordKind::Cname, "mailgun.org"),
            300,
        )
        .await
        .expect("a CNAME alone");
        let refused = add_record(
            &database,
            &zone,
            name(&zone, "email.mg"),
            data(RecordKind::Txt, "anything"),
            300,
        )
        .await;
        assert!(
            matches!(refused, Err(PlatformError::Refused(reason)) if reason == CNAME_STANDS_ALONE)
        );
    }

    /// Every change moves the serial: a secondary that sees the same one
    /// keeps the zone it had.
    #[tokio::test]
    async fn every_change_moves_the_serial_and_reads_back_whole() {
        let database = database().await;
        let site = project(&database, "site", None).await;
        let created = zone(&database, "teresaasprilla.com", &site)
            .await
            .expect("zone");

        let record = add_record(
            &database,
            &created,
            name(&created, "@"),
            data(RecordKind::Txt, "v=spf1 include:mailgun.org ~all"),
            3600,
        )
        .await
        .expect("txt");
        let after_add = find_by_name(&database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("there");
        assert!(after_add.serial > created.serial);
        assert_eq!(
            records(&database, &after_add).await.expect("records"),
            vec![record.clone()]
        );

        remove_record(&database, &after_add, &record.id)
            .await
            .expect("remove");
        let after_remove = find_by_name(&database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("there");
        assert!(after_remove.serial > after_add.serial);
        assert!(records(&database, &after_remove)
            .await
            .expect("records")
            .is_empty());

        assert_eq!(
            servers(&database, &created.id).await.expect("servers"),
            vec!["nd-self"]
        );
        delete(&database, &after_remove).await.expect("delete");
        assert!(find_by_name(&database, "teresaasprilla.com")
            .await
            .expect("read")
            .is_none());
    }
}
