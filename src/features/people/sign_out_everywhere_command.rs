use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::domain::values::account_event::AccountEvent;
use crate::infra::accounts::{devices, events, sessions};

/// End every session of a person and withdraw every device they trust:
/// what an administrator does when somebody's laptop is stolen.
pub struct SignOutEverywhere<'a> {
    pub by: &'a Account,
    pub person: &'a Account,
}

impl SignOutEverywhere<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        sessions::end_all(database, &self.person.id)
            .await
            .map_err(|error| error.to_string())?;
        devices::withdraw_all(database, &self.person.id)
            .await
            .map_err(|error| error.to_string())?;
        events::note(
            database,
            &self.person.id,
            AccountEvent::SignedOutEverywhere,
            &self.by.username,
            None,
            true,
        )
        .await;
        Ok(())
    }
}
