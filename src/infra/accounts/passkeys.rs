//! The passkeys an account signs in with: a public key each, for the host
//! it was made on.

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;
use wabot_addon_auth_passkey::{from_base64url, to_base64url, Registered};

use super::AccountResult;
use crate::domain::values::passkey_provider;
use crate::infra::clock::now_ms;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passkey {
    pub id: String,
    pub account_id: String,
    pub rp_id: String,
    pub public_key: Vec<u8>,
    pub sign_count: u32,
    pub name: String,
    pub synced: bool,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub aaguid: Option<String>,
    pub label: Option<String>,
    pub transports: String,
}

impl Passkey {
    /// The password manager or key that holds it, when it said.
    pub fn provider(&self) -> Option<&'static str> {
        self.aaguid.as_deref().and_then(passkey_provider::named)
    }

    /// What it is called: the name its owner gave it, else where it lives,
    /// else the browser it was made in.
    pub fn title(&self) -> &str {
        self.label
            .as_deref()
            .or(self.provider())
            .unwrap_or(&self.name)
    }

    /// A key somebody carries: reached over USB, NFC or Bluetooth, and
    /// never copied anywhere.
    pub fn security_key(&self) -> bool {
        !self.synced
            && self
                .transports
                .split(',')
                .any(|transport| matches!(transport, "usb" | "nfc" | "ble"))
    }
}

const COLUMNS: &str =
    "\"id\", \"account_id\", \"rp_id\", \"public_key\", \"sign_count\", \"name\", \
                       \"synced\", \"created_at\", \"last_used_at\", \"aaguid\", \"label\", \
                       \"transports\"";

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Passkey> {
    Ok(Passkey {
        id: row.get(0)?,
        account_id: row.get(1)?,
        rp_id: row.get(2)?,
        public_key: row.get(3)?,
        sign_count: row.get::<_, i64>(4)?.clamp(0, u32::MAX as i64) as u32,
        name: row.get(5)?,
        synced: row.get(6)?,
        created_at: row.get(7)?,
        last_used_at: row.get(8)?,
        aaguid: row.get(9)?,
        label: row.get(10)?,
        transports: row.get(11)?,
    })
}

pub async fn insert(
    database: &SqliteDatabase,
    account_id: &str,
    rp_id: &str,
    registered: &Registered,
    name: &str,
) -> AccountResult<Passkey> {
    let passkey = Passkey {
        id: to_base64url(&registered.credential_id),
        account_id: account_id.to_string(),
        rp_id: rp_id.to_string(),
        public_key: registered.public_key.clone(),
        sign_count: registered.sign_count,
        name: name.to_string(),
        synced: registered.backed_up,
        created_at: now_ms(),
        last_used_at: None,
        aaguid: Some(passkey_provider::spelled(&registered.aaguid))
            .filter(|aaguid| aaguid.bytes().any(|byte| byte != b'0' && byte != b'-')),
        label: None,
        transports: registered.transports.join(","),
    };
    let row = passkey.clone();
    database
        .write(move |connection| {
            connection.execute(
                &format!(
                    "INSERT INTO passkey ({COLUMNS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
                ),
                wabot::sqlite::rusqlite::params![
                    row.id,
                    row.account_id,
                    row.rp_id,
                    row.public_key,
                    row.sign_count as i64,
                    row.name,
                    row.synced,
                    row.created_at,
                    row.last_used_at,
                    row.aaguid,
                    row.label,
                    row.transports,
                ],
            )?;
            Ok(())
        })
        .await?;
    Ok(passkey)
}

pub async fn find(database: &SqliteDatabase, id: &str) -> AccountResult<Option<Passkey>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM passkey WHERE \"id\" = ?1"),
                    [id],
                    decode,
                )
                .optional()
        })
        .await?)
}

pub async fn of_account(
    database: &SqliteDatabase,
    account_id: &str,
) -> AccountResult<Vec<Passkey>> {
    let account = account_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM passkey WHERE \"account_id\" = ?1 ORDER BY \"created_at\""
                ))?
                .query_map([account], decode)?
                .collect()
        })
        .await?)
}

/// The credential ids an account holds for one host, as a ceremony lists
/// them.
pub async fn ids_for(
    database: &SqliteDatabase,
    account_id: &str,
    rp_id: &str,
) -> AccountResult<Vec<Vec<u8>>> {
    Ok(of_account(database, account_id)
        .await?
        .into_iter()
        .filter(|passkey| passkey.rp_id == rp_id)
        .filter_map(|passkey| from_base64url(&passkey.id).ok())
        .collect())
}

pub async fn used(
    database: &SqliteDatabase,
    id: &str,
    sign_count: u32,
    synced: bool,
) -> AccountResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE passkey SET \"sign_count\" = ?2, \"synced\" = ?3, \"last_used_at\" = ?4 \
                 WHERE \"id\" = ?1",
                (id, sign_count as i64, synced, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Call one of an account's passkeys something, or nothing again. True
/// when it was theirs.
pub async fn rename(
    database: &SqliteDatabase,
    account_id: &str,
    id: &str,
    label: Option<&str>,
) -> AccountResult<bool> {
    let (account, id, label) = (
        account_id.to_string(),
        id.to_string(),
        label.map(str::to_string),
    );
    Ok(database
        .write(move |connection| {
            Ok(connection.execute(
                "UPDATE passkey SET \"label\" = ?3 WHERE \"id\" = ?1 AND \"account_id\" = ?2",
                (id, account, label),
            )? > 0)
        })
        .await?)
}

/// Remove one of an account's passkeys. True when it was theirs.
pub async fn delete(database: &SqliteDatabase, account_id: &str, id: &str) -> AccountResult<bool> {
    let (account, id) = (account_id.to_string(), id.to_string());
    Ok(database
        .write(move |connection| {
            Ok(connection.execute(
                "DELETE FROM passkey WHERE \"id\" = ?1 AND \"account_id\" = ?2",
                (id, account),
            )? > 0)
        })
        .await?)
}

/// How many passkeys were made for a host, which is how many stop working
/// if it is renamed.
pub async fn count_for_host(database: &SqliteDatabase, rp_id: &str) -> AccountResult<usize> {
    let rp = rp_id.to_ascii_lowercase();
    Ok(database
        .read(move |connection| {
            connection.query_row(
                "SELECT COUNT(DISTINCT \"account_id\") FROM passkey WHERE \"rp_id\" = ?1",
                [rp],
                |row| row.get::<_, i64>(0),
            )
        })
        .await? as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn made_in_safari() -> Passkey {
        Passkey {
            id: "pk".into(),
            account_id: "a".into(),
            rp_id: "node.example.com".into(),
            public_key: Vec::new(),
            sign_count: 0,
            name: "Safari on macOS".into(),
            synced: true,
            created_at: 0,
            last_used_at: None,
            aaguid: Some("fbfc3007-154e-4ecc-8c0b-6e020557d7bd".into()),
            label: None,
            transports: "internal,hybrid".into(),
        }
    }

    /// Two passkeys made in Safari both read "Safari on macOS", which is
    /// how the page could not tell them apart. Where one lives is what
    /// its owner recognises, and a name they gave it beats both.
    #[test]
    fn a_passkey_is_called_by_its_owner_then_by_where_it_lives_then_by_its_browser() {
        let mut passkey = made_in_safari();
        assert_eq!(passkey.title(), "Apple Passwords");

        passkey.label = Some("Work laptop".into());
        assert_eq!(passkey.title(), "Work laptop");

        passkey.label = None;
        passkey.aaguid = None;
        assert_eq!(passkey.title(), "Safari on macOS");
    }

    #[test]
    fn a_key_reached_over_usb_and_never_copied_is_a_security_key() {
        let synced = made_in_safari();
        assert!(!synced.security_key());

        let yubikey = Passkey {
            synced: false,
            transports: "nfc,usb".into(),
            ..made_in_safari()
        };
        assert!(yubikey.security_key());

        let only_here = Passkey {
            synced: false,
            transports: "internal".into(),
            ..made_in_safari()
        };
        assert!(!only_here.security_key());
    }
}
