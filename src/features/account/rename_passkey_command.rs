use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::account::rename_passkey_request::RenamePasskeyRequest;
use crate::infra::accounts::passkeys;

const LONGEST: usize = 60;

pub struct RenamePasskey<'a> {
    pub account: &'a Account,
    pub asked: &'a RenamePasskeyRequest,
}

impl RenamePasskey<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        let label = self
            .asked
            .label
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if label.chars().count() > LONGEST {
            return Err(format!("a passkey's name is at most {LONGEST} characters"));
        }
        let renamed = passkeys::rename(
            database,
            &self.account.id,
            &self.asked.passkey_id,
            Some(label.as_str()).filter(|label| !label.is_empty()),
        )
        .await
        .map_err(|error| error.to_string())?;
        match renamed {
            true => Ok(()),
            false => Err("that passkey is not one of yours".into()),
        }
    }
}
