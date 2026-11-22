use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::people::sign_in_link_response::SignInLinkResponse;
use crate::infra::accounts::links::{self, Purpose};

/// What covers every case a passkey cannot: scripting off, an address
/// instead of a name, a lost device, a renamed domain. Its holder sees
/// who issued it in their activity once it is used.
pub struct IssueSignInLink<'a> {
    pub by: &'a Account,
    pub person: &'a Account,
}

impl IssueSignInLink<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<SignInLinkResponse, String> {
        let token = links::issue(
            database,
            &self.person.id,
            Purpose::SignIn,
            Some((&self.by.id, &self.by.username)),
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok(SignInLinkResponse {
            token,
            tenant_id: self.person.tenant_id.clone(),
        })
    }
}
