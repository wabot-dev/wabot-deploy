//! Invitations: how somebody who is not here becomes somebody who is.
//!
//! ## The invitee chooses their own password
//!
//! An administrator who types a colleague's password knows it, and it
//! travels over whatever channel they used to send it. An invitation
//! link carries no password at all — it carries the *right* to create
//! an account, once, before it expires. That is the whole reason this
//! exists rather than a "create user" form.
//!
//! Same mechanics as the setup token, for the same reasons: the token
//! is stored hashed, spent on use, and time-limited. A database
//! somebody reads is not a database somebody joins with.
//!
//! ## An invitation can carry a project
//!
//! Inviting somebody usually means inviting them *to* something. When
//! it names a project and a role, accepting puts them in it — so the
//! common case is one link rather than a link and then a second step
//! that somebody forgets.

use serde::Serialize;
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::roles::{NodeRole, ProjectRole};
use super::{now_ms, sha256_hex, Account, AccountError, AccountResult};

/// How long a link is worth anything.
///
/// A week: long enough to survive a weekend and a forgotten inbox,
/// short enough that a link in an old chat stops being a way in.
const INVITATION_DAYS: i64 = 7;

/// An invitation as the people page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Invitation {
    pub id: String,
    pub node_role: NodeRole,
    pub project_id: Option<String>,
    pub project_role: Option<ProjectRole>,
    pub created_by: String,
    pub expires_at: i64,
    pub used_at: Option<i64>,
    /// Which frontier it lets somebody into. `None` is the node's own.
    ///
    /// **Settled when the link is minted, never by whoever opens it.**
    /// An invitation that let the invitee choose their own scope would
    /// be an invitation to be anybody — see migration `0056`.
    pub tenant_id: Option<String>,
    /// The organisation's name, carried on the row rather than looked
    /// up by the page.
    ///
    /// The node's console lists these and must stay separable from
    /// tenancy — `the_node_console_does_not_know_what_an_organisation_is`
    /// fails the build if `people.rs` reaches for `platform::tenants`.
    /// A join here costs one column and keeps that true.
    pub scope_name: Option<String>,
}

impl Invitation {
    pub fn spent(&self) -> bool {
        self.used_at.is_some()
    }

    pub fn expired(&self, now: i64) -> bool {
        self.expires_at <= now
    }

    /// Still worth sending to somebody?
    pub fn live(&self, now: i64) -> bool {
        !self.spent() && !self.expired(now)
    }
}

/// Mint one, and return the token in clear.
///
/// The only time it exists in clear, on its way into a link the
/// inviter copies. What is stored is its hash.
/// Mint a link, naming the frontier the invitee lands in.
///
/// `None` is the node's own. Only the machine's operator may name one
/// that is not their own — that is how the first person of an
/// organisation gets in, and it is the only way in, since an
/// organisation with nobody in it cannot invite anybody either.
/// Only the hash is stored: a database somebody reads is not a list of
/// live links.
pub async fn create_into(
    database: &SqliteDatabase,
    created_by: &Account,
    node_role: NodeRole,
    project: Option<(&str, ProjectRole)>,
    tenant_id: Option<&str>,
) -> AccountResult<String> {
    let token = wabot::prelude::password::generate(32);
    // **Nobody invites into somebody else's frontier.** The machine's
    // operator may name any of them, because sharing the machine out is
    // what they do; anybody else gets their own and only their own. A
    // tenant's administrator minting a link into a rival organisation
    // would be the whole boundary undone by one form field.
    let scope = match created_by.is_admin() {
        true => tenant_id.map(str::to_string),
        false => created_by.tenant_id.clone(),
    };
    if !created_by.is_admin() && tenant_id != created_by.tenant_id.as_deref() {
        return Err(AccountError::Refused(
            "an invitation can only be into your own organisation".into(),
        ));
    }
    if let Some(id) = &scope {
        if crate::repositories::tenants::find(database, id)
            .await
            .map_err(|error| AccountError::Refused(error.to_string()))?
            .is_none()
        {
            return Err(AccountError::Refused("no such organisation".into()));
        }
    }

    // Only an administrator can make another one — **of that same
    // frontier**. Otherwise the weakest role is one invitation away from
    // the strongest, and nothing about the page would say so.
    //
    // Asked with `administers_scope` and not `is_admin`, because an
    // organisation's administrator has to be able to make a second one
    // of their own: `is_admin` is false for them by design, and using it
    // here would leave every organisation with exactly one
    // administrator for ever.
    if node_role == NodeRole::Admin && !created_by.administers_scope() {
        return Err(AccountError::Refused(
            "only an administrator can invite another administrator".into(),
        ));
    }

    let hash = sha256_hex(&token);
    let id = format!("inv-{}", wabot::prelude::password::generate(12));
    let expires_at = now_ms() + INVITATION_DAYS * 86_400_000;

    let (project_id, project_role) = match project {
        Some((id, role)) => (Some(id.to_string()), Some(role.as_str().to_string())),
        None => (None, None),
    };
    let creator = created_by.id.clone();

    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO invitation \
                   (\"id\", \"token_hash\", \"node_role\", \"project_id\", \"project_role\", \
                    \"created_by\", \"created_at\", \"expires_at\", \"tenant_id\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                (
                    id,
                    hash,
                    node_role.as_str(),
                    project_id,
                    project_role,
                    creator,
                    now_ms(),
                    expires_at,
                    scope,
                ),
            )?;
            Ok(())
        })
        .await?;

    Ok(token)
}

/// What this token is worth, if anything.
///
/// Read-only: the page behind the link needs to know what it is
/// offering before anybody fills the form in, and looking must not
/// spend it.
pub async fn look_up(database: &SqliteDatabase, token: &str) -> AccountResult<Option<Invitation>> {
    let hash = sha256_hex(token);
    let found: Option<Invitation> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT i.\"id\", i.\"node_role\", i.\"project_id\", i.\"project_role\", \
                     i.\"created_by\", i.\"expires_at\", i.\"used_at\", i.\"tenant_id\", \
                     t.\"name\" \
                     FROM invitation i \
                     LEFT JOIN tenant t ON t.\"id\" = i.\"tenant_id\" \
                     WHERE i.\"token_hash\" = ?1",
                    [hash],
                    |row| {
                        Ok(Invitation {
                            scope_name: row.get(8)?,
                            ..decode(row)?
                        })
                    },
                )
                .optional()
        })
        .await?;

    Ok(found.filter(|invitation| invitation.live(now_ms())))
}

/// Take the invitation, create the account, and put them where the
/// invitation said.
///
/// One call rather than three, because the three have to happen
/// together: an invitation spent on an account that failed to insert
/// is a link nobody can use again, and an account created without its
/// membership is somebody signed in to a node with nothing on it.
pub async fn accept(
    database: &SqliteDatabase,
    token: &str,
    username: &str,
    password: &str,
) -> AccountResult<Account> {
    let Some(invitation) = look_up(database, token).await? else {
        return Err(AccountError::Refused(
            "that invitation is not valid — it may have been used already, or expired".into(),
        ));
    };

    // The account first: it is the step that can be refused for
    // something the invitee can fix — a name taken, a password too
    // short — and spending the invitation on those would cost them the
    // link.
    // **In the frontier the invitation named**, which is the whole of
    // what this column exists for: before it, every account made here
    // was the node's own, so a tenant's console answered and nobody
    // could ever sign in to it.
    let account = super::create_in(
        database,
        username,
        password,
        invitation.node_role,
        invitation.tenant_id.as_deref(),
    )
    .await?;

    let (id, account_id) = (invitation.id.clone(), account.id.clone());
    let spent = database
        .write(move |connection| {
            connection.execute(
                "UPDATE invitation SET \"used_at\" = ?2, \"used_by\" = ?3 \
                 WHERE \"id\" = ?1 AND \"used_at\" IS NULL",
                (id, now_ms(), account_id),
            )
        })
        .await?;

    if spent == 0 {
        // Somebody else accepted between the look-up and here. The
        // account exists and has to go: leaving it would be an account
        // created by an invitation that was not spent on it.
        super::delete(database, &account.id).await?;
        return Err(AccountError::Refused(
            "that invitation was just used by somebody else".into(),
        ));
    }

    if let (Some(project_id), Some(role)) = (&invitation.project_id, invitation.project_role) {
        crate::policies::access::grant(database, &account.id, project_id, role)
            .await
            .map_err(|error| AccountError::Refused(error.to_string()))?;
    }

    tracing::info!(
        username = %account.username,
        role = account.role.as_str(),
        "invitation accepted"
    );
    Ok(account)
}

/// Every invitation, newest first.
pub async fn all(database: &SqliteDatabase) -> AccountResult<Vec<Invitation>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT i.\"id\", i.\"node_role\", i.\"project_id\", i.\"project_role\", \
                     i.\"created_by\", i.\"expires_at\", i.\"used_at\", i.\"tenant_id\", \
                     t.\"name\" \
                     FROM invitation i \
                     LEFT JOIN tenant t ON t.\"id\" = i.\"tenant_id\" \
                     ORDER BY i.\"created_at\" DESC",
                )?
                .query_map([], |row| {
                    Ok(Invitation {
                        scope_name: row.get(8)?,
                        ..decode(row)?
                    })
                })?
                .collect()
        })
        .await?)
}

/// Withdraw one that has not been used.
pub async fn revoke(database: &SqliteDatabase, id: &str) -> AccountResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM invitation WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Invitation> {
    Ok(Invitation {
        id: row.get(0)?,
        node_role: NodeRole::parse(&row.get::<_, String>(1)?),
        project_id: row.get(2)?,
        project_role: row
            .get::<_, Option<String>>(3)?
            .map(|role| ProjectRole::parse(&role)),
        created_by: row.get(4)?,
        expires_at: row.get(5)?,
        used_at: row.get(6)?,
        tenant_id: row.get(7)?,
        scope_name: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policies::access;

    async fn node() -> (SqliteDatabase, Account) {
        let database = crate::db::open_in_memory().await.expect("open");
        let token = super::super::issue_setup_token(&database)
            .await
            .expect("token");
        let admin =
            super::super::create_admin(&database, &token, "admin", "a long passphrase here")
                .await
                .expect("admin");
        (database, admin)
    }

    #[tokio::test]
    async fn an_invitation_creates_an_account_with_the_role_it_named() {
        let (database, admin) = node().await;
        let token = create_into(&database, &admin, NodeRole::Member, None, None)
            .await
            .expect("invited");

        let account = accept(&database, &token, "jorge", "another long passphrase")
            .await
            .expect("accepted");
        assert_eq!(account.role, NodeRole::Member);
        assert_eq!(account.username, "jorge");
    }

    /// A link into an organisation makes somebody of that organisation.
    ///
    /// **The door the frontier was missing.** Before this, a tenant's
    /// console answered on its own name, held its own certificate and
    /// refused a session from the wrong scope — and nobody could sign in
    /// to it, because every invitation made an account of the node's
    /// own. Verified on the node, and the way it showed was a sign-in
    /// page with nobody to sign in as.
    #[tokio::test]
    async fn a_link_into_an_organisation_makes_somebody_of_it() {
        let (database, admin) = node().await;
        let tenant = crate::repositories::tenants::create(&database, "Acme")
            .await
            .expect("tenant");

        let token = create_into(&database, &admin, NodeRole::Admin, None, Some(&tenant.id))
            .await
            .expect("invited");

        let account = accept(&database, &token, "jorge", "another long passphrase")
            .await
            .expect("accepted");

        assert_eq!(account.tenant_id.as_deref(), Some(&*tenant.id));
        // An administrator **of that organisation**, which is what the
        // link said — and not of the machine, which is the distinction
        // one clause in `is_admin` carries.
        assert!(account.administers_scope());
        assert!(
            !account.is_admin(),
            "an organisation's administrator was made an administrator of the machine"
        );
        assert!(account.belongs_to(Some(&tenant.id)));
        assert!(!account.belongs_to(None));
    }

    /// Nobody mints a link into somebody else's organisation.
    ///
    /// The machine's operator shares the machine out, so they may name
    /// any frontier. Anybody else gets their own and only their own —
    /// otherwise one form field undoes the whole boundary, and it would
    /// be a field the page draws for them.
    #[tokio::test]
    async fn nobody_invites_into_somebody_elses_organisation() {
        let (database, admin) = node().await;
        let theirs = crate::repositories::tenants::create(&database, "Acme")
            .await
            .expect("tenant");
        let rivals = crate::repositories::tenants::create(&database, "Globex")
            .await
            .expect("tenant");

        let token = create_into(&database, &admin, NodeRole::Admin, None, Some(&theirs.id))
            .await
            .expect("invited");
        let theirs_admin = accept(&database, &token, "acme", "another long passphrase")
            .await
            .expect("accepted");

        let refused = create_into(
            &database,
            &theirs_admin,
            NodeRole::Member,
            None,
            Some(&rivals.id),
        )
        .await
        .expect_err("refused");
        assert!(refused.to_string().contains("your own"), "{refused}");

        // Nor back out into the machine's own scope, which would be the
        // same hole in the other direction.
        let refused = create_into(&database, &theirs_admin, NodeRole::Member, None, None)
            .await
            .expect_err("refused");
        assert!(refused.to_string().contains("your own"), "{refused}");

        // Their own is fine, and administrators too: an organisation
        // that could never make a second administrator would have
        // exactly one for ever.
        assert!(create_into(
            &database,
            &theirs_admin,
            NodeRole::Admin,
            None,
            Some(&theirs.id)
        )
        .await
        .is_ok());
    }

    /// A link into an organisation that is not there is refused.
    #[tokio::test]
    async fn a_link_into_no_such_organisation_is_refused() {
        let (database, admin) = node().await;
        let refused = create_into(
            &database,
            &admin,
            NodeRole::Member,
            None,
            Some("tnt-nothing0001"),
        )
        .await
        .expect_err("refused");
        assert!(
            refused.to_string().contains("no such organisation"),
            "{refused}"
        );
    }

    /// The property this exists for: the token is worth one account.
    #[tokio::test]
    async fn an_invitation_works_once() {
        let (database, admin) = node().await;
        let token = create_into(&database, &admin, NodeRole::Member, None, None)
            .await
            .expect("invited");

        accept(&database, &token, "first", "a long passphrase here")
            .await
            .expect("accepted");
        let error = accept(&database, &token, "second", "a long passphrase here")
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("not valid"), "{error}");
    }

    #[tokio::test]
    async fn an_expired_invitation_is_not_valid() {
        let (database, admin) = node().await;
        let token = create_into(&database, &admin, NodeRole::Member, None, None)
            .await
            .expect("invited");

        let hash = sha256_hex(&token);
        database
            .write(move |connection| {
                connection.execute(
                    "UPDATE invitation SET \"expires_at\" = 1 WHERE \"token_hash\" = ?1",
                    [hash],
                )?;
                Ok(())
            })
            .await
            .expect("expire");

        assert!(look_up(&database, &token).await.expect("look up").is_none());
    }

    /// An invitation to a project puts them in it, so the common case
    /// is one link rather than a link and a second step somebody
    /// forgets.
    #[tokio::test]
    async fn an_invitation_to_a_project_lands_them_in_it() {
        let (database, admin) = node().await;
        let project = crate::repositories::projects::create(&database, "shared")
            .await
            .expect("project");

        let token = create_into(
            &database,
            &admin,
            NodeRole::Member,
            Some((&project.id, ProjectRole::Deployer)),
            None,
        )
        .await
        .expect("invited");

        let account = accept(&database, &token, "jorge", "a long passphrase here")
            .await
            .expect("accepted");

        let held = access::of(&database, &account, &project.id)
            .await
            .expect("access");
        assert!(held.may_deploy());
        assert!(!held.may_administer());
    }

    /// The weakest role on the node must not be one invitation away
    /// from the strongest.
    #[tokio::test]
    async fn a_member_cannot_invite_an_administrator() {
        let (database, admin) = node().await;
        let token = create_into(&database, &admin, NodeRole::Member, None, None)
            .await
            .expect("invited");
        let member = accept(&database, &token, "member", "a long passphrase here")
            .await
            .expect("accepted");

        let error = create_into(&database, &member, NodeRole::Admin, None, None)
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("administrator"), "{error}");

        // A member inviting a member is fine — that is how a project
        // owner brings somebody in.
        assert!(
            create_into(&database, &member, NodeRole::Member, None, None)
                .await
                .is_ok()
        );
    }

    /// A refusal the invitee can fix — a name already taken — must not
    /// cost them the link.
    #[tokio::test]
    async fn a_refused_signup_does_not_spend_the_invitation() {
        let (database, admin) = node().await;
        let token = create_into(&database, &admin, NodeRole::Member, None, None)
            .await
            .expect("invited");

        accept(&database, &token, "admin", "a long passphrase here")
            .await
            .expect_err("the name is taken");

        assert!(
            look_up(&database, &token).await.expect("look up").is_some(),
            "the invitation is still good"
        );
    }

    #[tokio::test]
    async fn a_revoked_invitation_stops_working() {
        let (database, admin) = node().await;
        let token = create_into(&database, &admin, NodeRole::Member, None, None)
            .await
            .expect("invited");
        let invitation = all(&database).await.expect("list").pop().expect("one");

        revoke(&database, &invitation.id).await.expect("revoked");
        assert!(look_up(&database, &token).await.expect("look up").is_none());
    }

    /// A database somebody reads must not be a database somebody joins
    /// with.
    #[tokio::test]
    async fn the_token_is_not_stored() {
        let (database, admin) = node().await;
        let token = create_into(&database, &admin, NodeRole::Member, None, None)
            .await
            .expect("invited");

        let stored: String = database
            .read(|connection| {
                connection.query_row("SELECT \"token_hash\" FROM invitation", [], |row| {
                    row.get(0)
                })
            })
            .await
            .expect("query");
        assert_ne!(stored, token);
        assert_eq!(stored, sha256_hex(&token));
    }

    #[tokio::test]
    async fn the_list_says_which_ones_are_spent() {
        let (database, admin) = node().await;
        let token = create_into(&database, &admin, NodeRole::Member, None, None)
            .await
            .expect("invited");
        create_into(&database, &admin, NodeRole::Member, None, None)
            .await
            .expect("invited");

        accept(&database, &token, "jorge", "a long passphrase here")
            .await
            .expect("accepted");

        let invitations = all(&database).await.expect("list");
        assert_eq!(invitations.len(), 2);
        assert_eq!(invitations.iter().filter(|i| i.spent()).count(), 1);
        assert_eq!(
            invitations.iter().filter(|i| i.live(now_ms())).count(),
            1,
            "the other is still worth sending"
        );
    }
}
