use crate::entities::account::Account;

/// Of the machine's own frontier rather than an organisation's.
///
/// Not a role: the shared pages (projects, services, databases) answer on
/// both consoles, and this is what picks which actor's use case a request
/// belongs to. What they may do inside a project is `crate::policies::access`.
pub fn belongs_here(account: &Account) -> bool {
    account.tenant_id.is_none()
}

pub fn manages_the_machine(account: &Account) -> bool {
    account.is_admin()
}

/// The same answer today and a different question, so the day they
/// diverge there is one place to change.
pub fn shares_the_machine_out(account: &Account) -> bool {
    manages_the_machine(account)
}
