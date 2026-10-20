//! A DNS zone's rows: the zone, its records, and which nodes answer for it.

use wabot::sqlite::rusqlite::Row;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::dns_record::DnsRecord;
use crate::domain::entities::dns_zone::{Delegation, DelegationStatus, DnsZone, DEFAULT_TTL};
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::policies::zone_serial;
use crate::domain::values::record_data::{RecordData, RecordKind};
use crate::domain::values::record_name::RecordName;
use crate::domain::values::zone_name::ZoneName;
use crate::infra::clock::now_ms;

pub const ALREADY_SERVED: &str = "this node already has a zone for that domain";
pub const OVERLAPS: &str =
    "that domain overlaps a zone somebody else manages on this node — a name has one owner";
pub const HOLDS_THE_NODE: &str =
    "that domain holds this node's own name, which is the node operator's to manage";
pub const HOLDS_AN_ORGANISATION: &str =
    "that domain holds the name another organisation's console answers to";
pub const CNAME_STANDS_ALONE: &str = "a CNAME stands alone: that name already has other records, \
     and a name with a CNAME can have nothing else";
pub const ALREADY_THERE: &str = "that record is already there";
pub const NO_SUCH_ZONE: &str = "there is no such zone";
pub const NO_SUCH_RECORD: &str = "there is no such record";

/// What creating a zone needs to know about who is asking — which owner
/// it will belong to — and nothing about whether they may.
pub struct NewZone<'a> {
    pub name: &'a ZoneName,
    pub tenant_id: Option<&'a str>,
    pub servers: &'a [String],
}

pub async fn create(database: &SqliteDatabase, asked: NewZone<'_>) -> PlatformResult<DnsZone> {
    let zones = all(database).await?;
    if zones.iter().any(|zone| zone.name == *asked.name) {
        return Err(PlatformError::Refused(ALREADY_SERVED.into()));
    }
    let overlapping = zones.iter().any(|zone| {
        let same_owner = zone.tenant_id.as_deref() == asked.tenant_id && zone.is_ours();
        !same_owner
            && (zone.name.holds(asked.name.as_str()) || asked.name.holds(zone.name.as_str()))
    });
    if overlapping {
        return Err(PlatformError::Refused(OVERLAPS.into()));
    }
    if let Some(tenant) = asked.tenant_id {
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
        tenant_id: asked.tenant_id.map(str::to_string),
        authority_id: None,
        serial: zone_serial::next(0, (now / 1000) as u64),
        default_ttl: DEFAULT_TTL,
        delegation: None,
        created_at: now,
        updated_at: now,
    };
    let row = zone.clone();
    let servers = asked.servers.to_vec();
    database
        .transaction(move |transaction| {
            transaction.execute(
                "INSERT INTO dns_zone (\"id\", \"name\", \"tenant_id\", \"authority_id\", \"serial\", \
                   \"default_ttl\", \"created_at\", \"updated_at\") \
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?6)",
                (
                    &row.id,
                    row.name.as_str(),
                    &row.tenant_id,
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

const ZONE_COLUMNS: &str = "SELECT \"id\", \"name\", \"tenant_id\", \"authority_id\", \"serial\", \
     \"default_ttl\", \"delegation_status\", \"delegation_note\", \"delegation_checked_at\", \
     \"created_at\", \"updated_at\" FROM dns_zone";

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
        (Some(status), note, Some(checked_at)) => Some(Delegation {
            status: DelegationStatus::parse(&status),
            note: note.unwrap_or_default(),
            checked_at,
        }),
        _ => None,
    };
    Ok(Some(DnsZone {
        id: row.get(0)?,
        name,
        tenant_id: row.get(2)?,
        authority_id: row.get(3)?,
        serial: row.get(4)?,
        default_ttl: row.get(5)?,
        delegation,
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

    fn zone_name(name: &str) -> ZoneName {
        ZoneName::parse(name).expect("zone")
    }

    async fn zone(
        database: &SqliteDatabase,
        name: &str,
        tenant: Option<&str>,
    ) -> PlatformResult<DnsZone> {
        create(
            database,
            NewZone {
                name: &zone_name(name),
                tenant_id: tenant,
                servers: &["nd-self".to_string()],
            },
        )
        .await
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
        zone(&database, "teresaasprilla.com", None)
            .await
            .expect("first");
        let second = zone(&database, "TeresaAsprilla.com.", None).await;
        assert!(matches!(second, Err(PlatformError::Refused(reason)) if reason == ALREADY_SERVED));
    }

    /// An organisation that could add `cloud.teresaasprilla.com` under
    /// somebody else's `teresaasprilla.com`, or the other way round, would
    /// be answering for names it does not own.
    #[tokio::test]
    async fn one_owner_cannot_reach_into_another_owners_zone() {
        let database = database().await;
        tenant(&database, "tnt-teresa", None).await;
        tenant(&database, "tnt-other", None).await;
        zone(&database, "teresaasprilla.com", Some("tnt-teresa"))
            .await
            .expect("hers");

        for owner in [Some("tnt-other"), None] {
            let refused = zone(&database, "cloud.teresaasprilla.com", owner).await;
            assert!(
                matches!(&refused, Err(PlatformError::Refused(reason)) if reason == OVERLAPS),
                "inside hers, for {owner:?}: {refused:?}"
            );
        }
        let refused = zone(&database, "asprilla.com", Some("tnt-other")).await;
        assert!(
            refused.is_ok(),
            "a different domain with similar letters: {refused:?}"
        );
        zone(&database, "blog.teresaasprilla.com", Some("tnt-teresa"))
            .await
            .expect("her own subdomain is hers");
    }

    #[tokio::test]
    async fn an_organisation_cannot_take_another_organisations_console_name() {
        let database = database().await;
        tenant(&database, "tnt-teresa", Some("cloud.teresaasprilla.com")).await;
        tenant(&database, "tnt-other", None).await;

        let refused = zone(&database, "teresaasprilla.com", Some("tnt-other")).await;
        assert!(
            matches!(refused, Err(PlatformError::Refused(reason)) if reason == HOLDS_AN_ORGANISATION)
        );
        zone(&database, "teresaasprilla.com", Some("tnt-teresa"))
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

        let refused = zone(&database, "nubebasura.lat", Some("tnt-other")).await;
        assert!(matches!(refused, Err(PlatformError::Refused(reason)) if reason == HOLDS_THE_NODE));
        zone(&database, "nubebasura.lat", None)
            .await
            .expect("the operator's own domain is the operator's");
    }

    #[tokio::test]
    async fn a_cname_stands_alone_at_its_name() {
        let database = database().await;
        let zone = zone(&database, "teresaasprilla.com", None)
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
        let created = zone(&database, "teresaasprilla.com", None)
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
