use crate::accounts::Account;
use crate::authz::node_user;
use crate::update::Catalogue;

pub async fn run(catalogue: &Catalogue, account: &Account) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    catalogue.refresh().await;
    Ok(())
}
