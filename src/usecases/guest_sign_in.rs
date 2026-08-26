use wabot::sqlite::SqliteDatabase;

use crate::accounts::{self, sessions};

/// **The frontier comes from the `Host`, not from the form.** A username
/// belongs to a scope, so three people called `jorge` is the ordinary
/// case and a search over the whole table would find whichever came
/// first — see migration `0057`.
pub async fn run(
    database: &SqliteDatabase,
    username: &str,
    password: &str,
    scope: Option<&str>,
) -> Result<String, String> {
    let Some(account) = accounts::authenticate(database, username, password, scope)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("that username and password do not match".into());
    };

    sessions::create(database, &account)
        .await
        .map_err(|error| error.to_string())
}
