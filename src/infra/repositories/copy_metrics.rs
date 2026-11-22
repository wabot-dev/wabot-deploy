//! Every `SELECT`, `INSERT` and `DELETE` a [`CopyMetric`] needs.

use wabot::sqlite::rusqlite::{params, Row};
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::copy_metric::CopyMetric;
use crate::domain::errors::PlatformResult;

const COLUMNS: &str = "\"container\", \"bucket\", \"samples\", \"running\", \"memory_max\", \
     \"memory_limit\", \"cpu_avg\", \"cpu_max\", \"cpu_limit\", \"throttled\", \"received\", \
     \"sent\", \"read\", \"written\", \"disk\", \"disk_partial\", \"disk_limit\"";

/// Written again rather than refused when the ten minutes are already
/// there: a process that restarted inside a bucket closes it a second time
/// with what it saw, and the later reading is the fuller one.
pub async fn write(database: &SqliteDatabase, rows: Vec<CopyMetric>) -> PlatformResult<()> {
    if rows.is_empty() {
        return Ok(());
    }
    database
        .write(move |connection| {
            let transaction = connection.unchecked_transaction()?;
            {
                let mut insert = transaction.prepare(&format!(
                    "INSERT OR REPLACE INTO copy_metric ({COLUMNS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"
                ))?;
                for row in &rows {
                    insert.execute(params![
                        row.container,
                        row.bucket,
                        row.samples,
                        row.running,
                        row.memory_max.map(|v| v as i64),
                        row.memory_limit.map(|v| v as i64),
                        row.cpu_avg,
                        row.cpu_max,
                        row.cpu_limit,
                        row.throttled,
                        row.received.map(|v| v as i64),
                        row.sent.map(|v| v as i64),
                        row.read.map(|v| v as i64),
                        row.written.map(|v| v as i64),
                        row.disk.map(|v| v as i64),
                        row.disk_partial,
                        row.disk_limit.map(|v| v as i64),
                    ])?;
                }
            }
            transaction.commit()?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn of_container(
    database: &SqliteDatabase,
    container: &str,
    from: i64,
    to: i64,
) -> PlatformResult<Vec<CopyMetric>> {
    let container = container.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM copy_metric \
                     WHERE \"container\" = ?1 AND \"bucket\" >= ?2 AND \"bucket\" < ?3 \
                     ORDER BY \"bucket\""
                ))?
                .query_map(params![container, from, to], decode)?
                .collect()
        })
        .await?)
}

pub async fn latest(database: &SqliteDatabase, container: &str) -> PlatformResult<Option<i64>> {
    let container = container.to_string();
    Ok(database
        .read(move |connection| {
            connection.query_row(
                "SELECT MAX(\"bucket\") FROM copy_metric WHERE \"container\" = ?1",
                [container],
                |row| row.get(0),
            )
        })
        .await?)
}

pub async fn forget_before(database: &SqliteDatabase, moment: i64) -> PlatformResult<usize> {
    Ok(database
        .write(move |connection| {
            connection.execute("DELETE FROM copy_metric WHERE \"bucket\" < ?1", [moment])
        })
        .await?)
}

fn decode(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<CopyMetric> {
    let bytes = |index: usize| -> wabot::sqlite::rusqlite::Result<Option<u64>> {
        Ok(row.get::<_, Option<i64>>(index)?.map(|v| v.max(0) as u64))
    };
    Ok(CopyMetric {
        container: row.get(0)?,
        bucket: row.get(1)?,
        samples: row.get(2)?,
        running: row.get(3)?,
        memory_max: bytes(4)?,
        memory_limit: bytes(5)?,
        cpu_avg: row.get(6)?,
        cpu_max: row.get(7)?,
        cpu_limit: row.get(8)?,
        throttled: row.get(9)?,
        received: bytes(10)?,
        sent: bytes(11)?,
        read: bytes(12)?,
        written: bytes(13)?,
        disk: bytes(14)?,
        disk_partial: row.get(15)?,
        disk_limit: bytes(16)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::open_in_memory;

    fn ten_minutes(container: &str, bucket: i64, running: u32) -> CopyMetric {
        CopyMetric {
            container: container.into(),
            bucket,
            samples: 10,
            running,
            memory_max: Some(5_000_000_000),
            cpu_avg: Some(120),
            disk: Some(42),
            disk_partial: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn ten_minutes_written_twice_keep_the_second_reading() {
        let database = open_in_memory().await.unwrap();
        write(&database, vec![ten_minutes("p.api", 600_000, 4)])
            .await
            .unwrap();
        write(&database, vec![ten_minutes("p.api", 600_000, 10)])
            .await
            .unwrap();
        let read = of_container(&database, "p.api", 0, 1_200_000)
            .await
            .unwrap();
        assert_eq!(read, vec![ten_minutes("p.api", 600_000, 10)]);
    }

    #[tokio::test]
    async fn a_copy_reads_only_its_own_ten_minutes_inside_the_window() {
        let database = open_in_memory().await.unwrap();
        write(
            &database,
            vec![
                ten_minutes("p.api", 0, 10),
                ten_minutes("p.api", 600_000, 10),
                ten_minutes("p.api", 1_200_000, 10),
                ten_minutes("p.api.2", 600_000, 10),
            ],
        )
        .await
        .unwrap();
        let read = of_container(&database, "p.api", 600_000, 1_200_000)
            .await
            .unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].bucket, 600_000);
        assert_eq!(forget_before(&database, 1_200_000).await.unwrap(), 3);
    }
}
