use wabot::sqlite::SqliteDatabase;

use crate::accounts::{self, sessions};

/// The first account on a node, made against the token `install`
/// printed. There is nobody to authorise it because there is nobody yet.
pub async fn run(
    database: &SqliteDatabase,
    setup_token: &str,
    username: &str,
    password: &str,
) -> Result<String, String> {
    let account = accounts::create_admin(database, setup_token, username, password)
        .await
        .map_err(|error| error.to_string())?;

    sessions::create(database, &account)
        .await
        .map_err(|error| error.to_string())
}
