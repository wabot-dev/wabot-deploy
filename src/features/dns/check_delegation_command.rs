use std::net::IpAddr;

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::dns_zone::{verdict, Delegation, DelegationStatus, DnsZone};
use crate::infra::dns::delegation::{self, Asker, Outcome};
use crate::infra::repositories::zones;

/// Ask the parent zone where it sends resolvers for this one, and keep the
/// answer on the zone. Only for a zone administered here: the node that
/// owns it is the one whose page says what to fix.
pub struct CheckDelegation<'a> {
    pub zone: &'a DnsZone,
}

impl CheckDelegation<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        self.run_with(
            database,
            &delegation::OverTheNetwork,
            &delegation::system_resolvers(),
        )
        .await
    }

    pub async fn run_with(
        &self,
        database: &SqliteDatabase,
        asker: &dyn Asker,
        resolvers: &[IpAddr],
    ) -> Result<(), String> {
        let zone = self.zone;
        if !zone.is_ours() {
            return Ok(());
        }
        let servers = zones::servers(database, &zone.id)
            .await
            .map_err(|error| error.to_string())?;
        let answering = crate::infra::dns::name_servers(database, &servers).await?;

        let checked_at = crate::infra::clock::now_ms();
        let seen = match delegation::delegated(asker, resolvers, zone.name.as_str()).await {
            Outcome::Delegated(names) => Delegation {
                status: verdict(&names, &answering),
                delegated: names,
                reason: None,
                checked_at,
            },
            Outcome::Unregistered => Delegation {
                status: DelegationStatus::Unregistered,
                delegated: Vec::new(),
                reason: None,
                checked_at,
            },
            Outcome::Unanswered(reason) => Delegation {
                status: DelegationStatus::Unknown,
                delegated: Vec::new(),
                reason: Some(reason),
                checked_at,
            },
        };
        zones::save_delegation(database, zone, &seen)
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::zone_name::ZoneName;
    use hickory_server::proto::op::Message;
    use hickory_server::proto::rr::RecordType;

    struct NobodyAnswers;

    #[async_trait::async_trait]
    impl Asker for NobodyAnswers {
        async fn ask(
            &self,
            server: IpAddr,
            _: &str,
            _: RecordType,
            _: bool,
        ) -> Result<Message, String> {
            Err(format!("{server} did not answer"))
        }
    }

    /// What was learned is kept on the zone, reason and all, so the page
    /// and `doctor` say it without asking again.
    #[tokio::test]
    async fn what_the_check_learned_stays_on_the_zone() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let name = ZoneName::parse("teresaasprilla.com").expect("zone");
        let zone = zones::create(
            &database,
            zones::NewZone {
                name: &name,
                tenant_id: None,
                servers: &[],
            },
        )
        .await
        .expect("zone");

        CheckDelegation { zone: &zone }
            .run_with(
                &database,
                &NobodyAnswers,
                &["192.0.2.53".parse().expect("ip")],
            )
            .await
            .expect("kept");

        let seen = zones::find_by_name(&database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("there")
            .delegation
            .expect("a delegation");
        assert_eq!(seen.status, DelegationStatus::Unknown);
        assert!(
            seen.reason
                .as_deref()
                .unwrap_or_default()
                .contains("192.0.2.53"),
            "{seen:?}"
        );
    }
}
