//! Who may operate this node, and how they prove it.
//!
//! ## The setup token, and the race it closes
//!
//! The node has a publicly trusted certificate the moment `install`
//! finishes. A console that simply offered "create the first admin"
//! would be offering it to whoever reaches the hostname first, and the
//! operator is not necessarily the fastest.
//!
//! So `install` prints a token and stores its hash, and the setup page
//! wants that token as well as a password. The operator is already
//! looking at the terminal that printed it; an attacker is not. It is
//! what Jenkins does with `initialAdminPassword`, for the same reason.
//!
//! The token is spent on use. A second setup attempt has nothing to
//! offer.
//!
//! ## Sessions are rows, not signed tokens
//!
//! A JWT in a cookie needs no server state, and cannot be revoked
//! without adding some. SQLite is already open here, so the lookup
//! costs microseconds — and on a box that deploys containers, "log
//! that session out now" is worth more than saving a local read.

pub mod invitations;
pub mod roles;
pub mod sessions;

use serde::{Deserialize, Serialize};
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

/// How long the setup token is worth anything.
///
/// Not forever: a node installed and forgotten for a month should not
/// still be offering its console to whoever finds the token in an old
/// terminal buffer.
const SETUP_TOKEN_HOURS: i64 = 24;

const SETUP_TOKEN_KEY: &str = "setup_token_hash";
const SETUP_TOKEN_EXPIRY_KEY: &str = "setup_token_expires_at";

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("storage: {0}")]
    Storage(#[from] wabot::sqlite::SqliteError),
    #[error("hashing: {0}")]
    Hash(String),
    #[error("{0}")]
    Refused(String),
}

pub type AccountResult<T> = Result<T, AccountError>;

/// How a refusal reaches an HTTP caller.
///
/// A `Refused` is something the operator typed — 400, with the message
/// they need. Everything else is the node's own fault and says nothing
/// beyond that: a storage error's text is an internal path, and an
/// argon2 error is a parameter mismatch nobody outside can act on.
impl From<AccountError> for wabot::rest::RestError {
    fn from(error: AccountError) -> Self {
        match error {
            AccountError::Refused(message) => wabot::rest::RestError::Client {
                status: 400,
                message,
            },
            other => {
                tracing::error!(error = %other, "account operation failed");
                wabot::rest::RestError::Internal("account operation failed".into())
            }
        }
    }
}

/// Serializable because this is also what travels through `Auth`: the
/// console's claims are the account, and a second struct of the same
/// shape is a second place for it to drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub username: String,
    /// What they are on the node. Travels in the session claims, so a
    /// handler knows without a second query — and a role changed while
    /// somebody is signed in takes effect on their next request, since
    /// the middleware reads the row every time.
    pub role: roles::NodeRole,
    /// Which theme they read in. Travels with the account so the
    /// choice follows the person rather than the browser.
    pub theme: crate::console::shell::Theme,
    /// Which language they read in. Beside the theme and for the same
    /// reason: a preference that does not follow somebody between
    /// machines is one they set again on each.
    pub language: crate::console::language::Language,
}

impl Account {
    pub fn is_admin(&self) -> bool {
        self.role == roles::NodeRole::Admin
    }
}

/// Is there anybody yet?
///
/// What the console asks on every request to decide between the setup
/// page and the login page.
pub async fn any_account(database: &SqliteDatabase) -> AccountResult<bool> {
    let count: i64 = database
        .read(|connection| {
            connection.query_row("SELECT COUNT(*) FROM account", [], |row| row.get(0))
        })
        .await?;
    Ok(count > 0)
}

// ---------- the setup token -------------------------------------------

/// Mint a setup token, replacing any previous one.
///
/// Returns the token in clear — the only time it exists in clear — for
/// `install` to print. What is stored is its hash, so a database
/// someone reads is not a database someone sets up.
pub async fn issue_setup_token(database: &SqliteDatabase) -> AccountResult<String> {
    // 32 characters of the framework's alphanumeric generator. Long
    // enough that guessing is not a strategy, short enough to
    // copy-paste from a terminal without wrapping.
    let token = wabot::prelude::password::generate(32);
    let expires_at = now_ms() + SETUP_TOKEN_HOURS * 3_600_000;

    put_setting(database, SETUP_TOKEN_KEY, &sha256_hex(&token)).await?;
    put_setting(database, SETUP_TOKEN_EXPIRY_KEY, &expires_at.to_string()).await?;
    Ok(token)
}

/// Is a usable setup token outstanding?
pub async fn setup_token_valid(database: &SqliteDatabase) -> AccountResult<bool> {
    let Some(expires_at) = get_setting(database, SETUP_TOKEN_EXPIRY_KEY).await? else {
        return Ok(false);
    };
    Ok(expires_at.parse::<i64>().unwrap_or(0) > now_ms())
}

async fn spend_setup_token(database: &SqliteDatabase, offered: &str) -> AccountResult<()> {
    let Some(stored) = get_setting(database, SETUP_TOKEN_KEY).await? else {
        return Err(AccountError::Refused(
            "this node has no setup token — run `wabot-deploy install` to issue one".into(),
        ));
    };
    if !setup_token_valid(database).await? {
        return Err(AccountError::Refused(
            "the setup token has expired — run `wabot-deploy install` to issue another".into(),
        ));
    }

    // Constant-time, so the comparison does not leak the token one
    // character at a time to somebody measuring.
    if !constant_time_eq(&sha256_hex(offered), &stored) {
        return Err(AccountError::Refused("that is not the setup token".into()));
    }

    // Spent. A second setup attempt has nothing to offer, which is the
    // property that makes this a one-shot rather than a password.
    delete_setting(database, SETUP_TOKEN_KEY).await?;
    delete_setting(database, SETUP_TOKEN_EXPIRY_KEY).await?;
    Ok(())
}

// ---------- accounts ---------------------------------------------------

/// Create the first administrator.
///
/// Refuses if one exists — this is setup, not registration, and a node
/// that let a second person "set up" would be a node anyone could take
/// over by asking twice.
pub async fn create_admin(
    database: &SqliteDatabase,
    setup_token: &str,
    username: &str,
    password: &str,
) -> AccountResult<Account> {
    if any_account(database).await? {
        return Err(AccountError::Refused(
            "this node already has an account; sign in instead".into(),
        ));
    }

    let username = username.trim();
    validate_username(username)?;
    validate_password(password)?;

    // Checked *after* validation so a bad password does not spend the
    // token, and *before* the insert so a failed insert does not leave
    // it spent.
    spend_setup_token(database, setup_token).await?;

    let account = insert(database, username, password, roles::NodeRole::Admin).await?;
    tracing::info!(username = %account.username, "administrator created");
    Ok(account)
}

/// Create somebody who is not the first.
///
/// No setup token: this is the invitation path, and the invitation is
/// what was checked. Refuses a name already taken, which the unique
/// index enforces and this turns into something readable.
pub async fn create(
    database: &SqliteDatabase,
    username: &str,
    password: &str,
    role: roles::NodeRole,
) -> AccountResult<Account> {
    let username = username.trim();
    validate_username(username)?;
    validate_password(password)?;

    let account = insert(database, username, password, role).await?;
    tracing::info!(username = %account.username, role = role.as_str(), "account created");
    Ok(account)
}

async fn insert(
    database: &SqliteDatabase,
    username: &str,
    password: &str,
    role: roles::NodeRole,
) -> AccountResult<Account> {
    let account = Account {
        theme: crate::console::shell::Theme::System,
        language: crate::console::language::Language::En,
        id: format!("acc-{}", wabot::prelude::password::generate(16)),
        username: username.to_string(),
        role,
    };
    let hash = wabot::prelude::password::hash(password)
        .map_err(|error| AccountError::Hash(error.to_string()))?;

    let (id, name) = (account.id.clone(), account.username.clone());
    let taken = account.username.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO account (\"id\", \"username\", \"password_hash\", \"role\", \
                                    \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (id, name, hash, role.as_str(), now_ms()),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| {
            if error.to_string().contains("UNIQUE") {
                AccountError::Refused(format!("the name {taken:?} is taken"))
            } else {
                AccountError::Storage(error)
            }
        })?;

    Ok(account)
}

/// Everybody on the node, for the people page.
pub async fn all(database: &SqliteDatabase) -> AccountResult<Vec<Account>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"username\", \"role\", \"theme\", \"language\" \
                     FROM account ORDER BY \"username\"",
                )?
                .query_map([], decode)?
                .collect()
        })
        .await?)
}

/// Change what somebody is on the node.
pub async fn set_role(
    database: &SqliteDatabase,
    account_id: &str,
    role: roles::NodeRole,
) -> AccountResult<()> {
    // The last administrator cannot demote themselves. A node with no
    // administrator has nobody who can create one, and the way back is
    // editing the database by hand.
    if role != roles::NodeRole::Admin && administrators(database).await? <= 1 {
        let is_admin = all(database)
            .await?
            .into_iter()
            .any(|account| account.id == account_id && account.is_admin());
        if is_admin {
            return Err(AccountError::Refused(
                "this is the node's only administrator — make somebody else one first".into(),
            ));
        }
    }

    let id = account_id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE account SET \"role\" = ?2 WHERE \"id\" = ?1",
                (id, role.as_str()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Remove somebody, and everything that was theirs alone.
pub async fn delete(database: &SqliteDatabase, account_id: &str) -> AccountResult<()> {
    if administrators(database).await? <= 1
        && all(database)
            .await?
            .into_iter()
            .any(|account| account.id == account_id && account.is_admin())
    {
        return Err(AccountError::Refused(
            "this is the node's only administrator — make somebody else one first".into(),
        ));
    }

    let id = account_id.to_string();
    database
        .write(move |connection| {
            // Sessions and memberships cascade; the account row is the
            // one thing to remove.
            connection.execute("DELETE FROM account WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

async fn administrators(database: &SqliteDatabase) -> AccountResult<usize> {
    Ok(all(database)
        .await?
        .into_iter()
        .filter(Account::is_admin)
        .count())
}

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        username: row.get(1)?,
        role: roles::NodeRole::parse(&row.get::<_, String>(2)?),
        theme: crate::console::shell::Theme::parse(&row.get::<_, String>(3)?),
        language: crate::console::language::Language::parse(&row.get::<_, String>(4)?),
    })
}

/// Store which theme somebody reads in.
pub async fn set_theme(
    database: &SqliteDatabase,
    id: &str,
    theme: crate::console::shell::Theme,
) -> AccountResult<()> {
    let (id, theme) = (id.to_string(), theme.as_str().to_string());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE account SET \"theme\" = ?2 WHERE \"id\" = ?1",
                (id, theme),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Store which language somebody reads in.
pub async fn set_language(
    database: &SqliteDatabase,
    id: &str,
    language: crate::console::language::Language,
) -> AccountResult<()> {
    let (id, language) = (id.to_string(), language.as_str().to_string());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE account SET \"language\" = ?2 WHERE \"id\" = ?1",
                (id, language),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Check a username and password.
///
/// `None` for both "no such user" and "wrong password", deliberately:
/// telling them apart tells an attacker which usernames exist.
/// Replace an account's password with a freshly generated one.
///
/// **Generated rather than taken as an argument**, and the difference is
/// not politeness: a password on a command line lands in shell history, in
/// the process table while it runs, and in whatever the terminal scrolled
/// past. This one is printed once and stored as a hash like every other.
///
/// Matched case-insensitively, the way signing in matches it — an operator
/// recovering an account should not have to remember how they capitalised
/// it.
pub async fn reset_password(
    database: &SqliteDatabase,
    username: &str,
) -> AccountResult<(String, String)> {
    let lookup = username.trim().to_lowercase();
    // Long enough that it is not worth attacking, and made of the same
    // alphabet as every other secret this node prints so it survives being
    // copied out of a terminal.
    let password = wabot::prelude::password::generate(24);
    let hash = wabot::prelude::password::hash(&password)
        .map_err(|error| AccountError::Hash(error.to_string()))?;

    let (found, stored) = (lookup.clone(), hash);
    let name: Option<String> = database
        .write(move |connection| {
            let name: Option<String> = connection
                .query_row(
                    "SELECT \"username\" FROM account WHERE lower(\"username\") = ?1",
                    [&found],
                    |row| row.get(0),
                )
                .optional()?;
            if name.is_some() {
                connection.execute(
                    "UPDATE account SET \"password_hash\" = ?2 WHERE lower(\"username\") = ?1",
                    (&found, &stored),
                )?;
            }
            Ok(name)
        })
        .await?;

    match name {
        Some(name) => Ok((name, password)),
        None => Err(AccountError::Refused(format!(
            "no account here is called {username}"
        ))),
    }
}

pub async fn authenticate(
    database: &SqliteDatabase,
    username: &str,
    password: &str,
) -> AccountResult<Option<Account>> {
    let lookup = username.trim().to_lowercase();
    let row: Option<(String, String, String, String, String, String)> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"id\", \"username\", \"password_hash\", \"role\", \"theme\", \
                     \"language\" FROM account WHERE lower(\"username\") = ?1",
                    [lookup],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .optional()
        })
        .await?;

    let Some((id, username, hash, role, theme, language)) = row else {
        // Hash anyway. Returning early on an unknown username makes
        // the response measurably faster, and that difference is a
        // list of which usernames exist.
        let _ = wabot::prelude::password::verify(password, DUMMY_HASH);
        return Ok(None);
    };

    if !wabot::prelude::password::verify(password, &hash) {
        return Ok(None);
    }

    let touched = id.clone();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE account SET \"last_seen_at\" = ?2 WHERE \"id\" = ?1",
                (touched, now_ms()),
            )?;
            Ok(())
        })
        .await?;

    Ok(Some(Account {
        theme: crate::console::shell::Theme::parse(&theme),
        language: crate::console::language::Language::parse(&language),
        id,
        username,
        role: roles::NodeRole::parse(&role),
    }))
}

/// A real argon2id hash of a value nobody knows, so the unknown-user
/// path costs what the known-user path costs.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHR2YWx1ZQ$\
                          7Nn2yQqQzHkZ0ZQqPZzZ0ZQqPZzZ0ZQqPZzZ0ZQqPZo";

fn validate_username(username: &str) -> AccountResult<()> {
    if username.len() < 2 || username.len() > 40 {
        return Err(AccountError::Refused(
            "a username is between 2 and 40 characters".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(AccountError::Refused(
            "a username holds letters, digits, and - _ .".into(),
        ));
    }
    Ok(())
}

/// Length, and nothing else.
///
/// No character-class rules: they push people towards `Password1!`,
/// which is worse than a long phrase. Twelve is the floor because this
/// console can start containers on the machine.
fn validate_password(password: &str) -> AccountResult<()> {
    if password.chars().count() < 12 {
        return Err(AccountError::Refused(
            "a password needs at least 12 characters — a phrase is easier to remember \
             and harder to guess than a short one with punctuation in it"
                .into(),
        ));
    }
    if password.chars().count() > 200 {
        return Err(AccountError::Refused("that password is too long".into()));
    }
    Ok(())
}

// ---------- settings ---------------------------------------------------

async fn get_setting(database: &SqliteDatabase, key: &str) -> AccountResult<Option<String>> {
    let key = key.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"value\" FROM setting WHERE \"key\" = ?1",
                    [key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
        })
        .await?)
}

async fn put_setting(database: &SqliteDatabase, key: &str, value: &str) -> AccountResult<()> {
    let (key, value) = (key.to_string(), value.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO setting (\"key\", \"value\", \"updated_at\") VALUES (?1, ?2, ?3) \
                 ON CONFLICT (\"key\") DO UPDATE SET \
                   \"value\" = excluded.\"value\", \"updated_at\" = excluded.\"updated_at\"",
                (key, value, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

async fn delete_setting(database: &SqliteDatabase, key: &str) -> AccountResult<()> {
    let key = key.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM setting WHERE \"key\" = ?1", [key])?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub(crate) fn sha256_hex(value: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Compare without letting the time taken say how much matched.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |differences, (a, b)| differences | (a ^ b))
        == 0
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::db::open_in_memory().await.expect("open")
    }

    #[tokio::test]
    async fn a_fresh_node_has_nobody() {
        assert!(!any_account(&database().await).await.expect("query"));
    }

    #[tokio::test]
    async fn the_admin_is_created_with_a_valid_token() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");

        let account = create_admin(&database, &token, "jorge", "correct horse battery")
            .await
            .expect("created");
        assert_eq!(account.username, "jorge");
        assert!(any_account(&database).await.expect("query"));
    }

    /// The way back into a node whose only administrator forgot their
    /// password — which, until this existed, was a reinstall.
    ///
    /// The old password stops working in the same breath, because the
    /// point is recovery and not a second key: an account with two
    /// passwords is one whose owner cannot tell whether the one they had
    /// was stolen.
    #[tokio::test]
    async fn a_forgotten_password_can_be_replaced_and_the_old_one_stops() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");
        create_admin(&database, &token, "Jorge", "correct horse battery")
            .await
            .expect("created");

        // Matched the way signing in matches it: an operator recovering an
        // account should not have to remember how they capitalised it.
        let (name, password) = reset_password(&database, "jorge").await.expect("reset");
        assert_eq!(name, "Jorge", "and the answer says whose it was");

        assert!(authenticate(&database, "jorge", &password)
            .await
            .expect("query")
            .is_some());
        assert!(
            authenticate(&database, "jorge", "correct horse battery")
                .await
                .expect("query")
                .is_none(),
            "the password they lost is not still a way in"
        );
    }

    /// A name nobody has is refused rather than quietly creating one: the
    /// setup token is the only thing that mints an account, and a typo
    /// here must not become a second administrator.
    #[tokio::test]
    async fn resetting_a_name_that_is_not_here_makes_nobody() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");
        create_admin(&database, &token, "jorge", "correct horse battery")
            .await
            .expect("created");

        assert!(reset_password(&database, "someone-else").await.is_err());
        assert_eq!(all(&database).await.expect("query").len(), 1);
    }

    /// The whole reason the token exists. Without it, whoever reaches
    /// the hostname first is the administrator.
    #[tokio::test]
    async fn the_wrong_token_creates_nobody() {
        let database = database().await;
        issue_setup_token(&database).await.expect("token");

        let error = create_admin(
            &database,
            "not-the-token",
            "mallory",
            "a long enough password",
        )
        .await
        .expect_err("refused");
        assert!(error.to_string().contains("setup token"), "{error}");
        assert!(!any_account(&database).await.expect("query"));
    }

    /// One shot. A token that stayed valid after use would be a
    /// password nobody chose.
    #[tokio::test]
    async fn the_token_is_spent_on_use() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");
        create_admin(&database, &token, "jorge", "correct horse battery")
            .await
            .expect("created");

        assert!(!setup_token_valid(&database).await.expect("query"));
    }

    /// Setup is not registration: a second person cannot set the node
    /// up, even holding a token.
    #[tokio::test]
    async fn only_the_first_account_is_created_this_way() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");
        create_admin(&database, &token, "jorge", "correct horse battery")
            .await
            .expect("created");

        let second = issue_setup_token(&database).await.expect("token");
        let error = create_admin(&database, &second, "mallory", "another long password")
            .await
            .expect_err("refused");
        assert!(
            error.to_string().contains("already has an account"),
            "{error}"
        );
    }

    /// A rejected password must not cost the token — otherwise a typo
    /// means re-running install.
    #[tokio::test]
    async fn a_bad_password_does_not_spend_the_token() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");

        assert!(create_admin(&database, &token, "jorge", "short")
            .await
            .is_err());
        assert!(setup_token_valid(&database).await.expect("query"));

        // And the same token still works.
        create_admin(&database, &token, "jorge", "correct horse battery")
            .await
            .expect("created on the second try");
    }

    #[tokio::test]
    async fn an_expired_token_is_refused() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");
        put_setting(&database, SETUP_TOKEN_EXPIRY_KEY, "1")
            .await
            .expect("expire it");

        let error = create_admin(&database, &token, "jorge", "correct horse battery")
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("expired"), "{error}");
    }

    #[tokio::test]
    async fn the_right_password_signs_in_and_the_wrong_one_does_not() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");
        create_admin(&database, &token, "Jorge", "correct horse battery")
            .await
            .expect("created");

        assert!(authenticate(&database, "Jorge", "correct horse battery")
            .await
            .expect("query")
            .is_some());
        // Case-insensitive: somebody typing at 3am should get in.
        assert!(authenticate(&database, "jorge", "correct horse battery")
            .await
            .expect("query")
            .is_some());
        assert!(authenticate(&database, "jorge", "wrong")
            .await
            .expect("query")
            .is_none());
        assert!(authenticate(&database, "nobody", "correct horse battery")
            .await
            .expect("query")
            .is_none());
    }

    /// The password is never stored, and the hash is argon2id rather
    /// than something faster.
    #[tokio::test]
    async fn the_password_is_not_in_the_database() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");
        create_admin(&database, &token, "jorge", "correct horse battery")
            .await
            .expect("created");

        let stored: String = database
            .read(|connection| {
                connection.query_row("SELECT \"password_hash\" FROM account", [], |row| {
                    row.get(0)
                })
            })
            .await
            .expect("query");

        assert!(!stored.contains("correct horse battery"));
        assert!(stored.starts_with("$argon2id$"), "{stored}");
    }

    /// Nor is the setup token: a database someone reads must not be a
    /// database someone sets up.
    #[tokio::test]
    async fn the_setup_token_is_not_in_the_database() {
        let database = database().await;
        let token = issue_setup_token(&database).await.expect("token");

        let stored = get_setting(&database, SETUP_TOKEN_KEY)
            .await
            .expect("query")
            .expect("present");
        assert_ne!(stored, token);
        assert_eq!(stored, sha256_hex(&token));
    }

    #[test]
    fn passwords_are_judged_on_length_alone() {
        assert!(validate_password("correct horse battery").is_ok());
        assert!(validate_password("aaaaaaaaaaaa").is_ok(), "12 is the floor");
        assert!(
            validate_password("Sh0rt!").is_err(),
            "punctuation is not length"
        );
        assert!(validate_password(&"x".repeat(500)).is_err());
    }

    #[test]
    fn usernames_are_restricted_to_what_reads_back() {
        assert!(validate_username("jorge").is_ok());
        assert!(validate_username("jorge.narvaez_1").is_ok());
        assert!(validate_username("a").is_err());
        assert!(validate_username("has space").is_err());
        assert!(validate_username("<script>").is_err());
    }

    #[test]
    fn the_comparison_does_not_short_circuit() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
    }
}
