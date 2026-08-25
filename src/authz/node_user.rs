use crate::accounts::Account;

pub fn manages_the_machine(account: &Account) -> bool {
    account.is_admin()
}

/// The same answer today and a different question, so the day they
/// diverge there is one place to change.
pub fn shares_the_machine_out(account: &Account) -> bool {
    manages_the_machine(account)
}
