use crate::domain::entities::account::Account;

pub fn belongs_here(account: &Account) -> bool {
    account.tenant_id.is_some()
}

pub fn administers(account: &Account) -> bool {
    belongs_here(account) && account.administers_scope()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::roles::NodeRole;

    fn account(tenant: Option<&str>, role: NodeRole) -> Account {
        Account {
            id: "acc-1".into(),
            username: "jorge".into(),
            role,
            theme: crate::domain::values::theme::Theme::System,
            language: crate::domain::values::language::Language::En,
            tenant_id: tenant.map(str::to_string),
        }
    }

    #[test]
    fn the_machines_operator_is_not_one_of_them() {
        assert!(!belongs_here(&account(None, NodeRole::Admin)));
        assert!(!administers(&account(None, NodeRole::Admin)));

        assert!(belongs_here(&account(Some("tnt-acme"), NodeRole::Member)));
        assert!(administers(&account(Some("tnt-acme"), NodeRole::Admin)));
        assert!(!administers(&account(Some("tnt-acme"), NodeRole::Member)));
    }
}
