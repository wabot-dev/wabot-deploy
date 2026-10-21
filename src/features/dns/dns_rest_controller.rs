use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::shell::auth::{
    back_with_error, read_form, see_other, segments, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

/// The doors that are the machine's to open: the names this node answers
/// as, which nodes answer for a zone, and putting down a zone no project
/// holds. Registered on the machine's console only, so an organisation's
/// has none of them — a 404 rather than a refusal somebody has to remember.
#[injectable]
pub struct DnsRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

const TAB: &str = "/node/dns";

#[rest_controller("/")]
impl DnsRestController {
    #[post("/node/dns/names")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_names(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = crate::features::dns::set_name_servers_request::SetNameServersRequest {
            names: form
                .get("names")
                .map(|typed| {
                    typed
                        .split(|letter: char| letter.is_whitespace() || letter == ',')
                        .filter(|name| !name.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(TAB, &reason.to_string()));
        }
        match crate::features::dns::node_user_set_name_servers::run(
            &self.state.database,
            &self.state.container,
            &account,
            &asked,
        )
        .await
        {
            Ok(()) => Ok(see_other(TAB)),
            Err(reason) => Ok(back_with_error(TAB, &reason)),
        }
    }

    #[post("/node/dns/:zone/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn evict_zone(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(zone) = segments(&path).get(2).map(|zone| zone.to_string()) else {
            return Ok(see_other(TAB));
        };
        match crate::features::dns::node_user_evict_zone::run(
            &self.state.database,
            &self.state.container,
            &account,
            &zone,
        )
        .await
        {
            Ok(()) => Ok(see_other(TAB)),
            Err(reason) => Ok(back_with_error(TAB, &reason)),
        }
    }

    #[post("/projects/:project/zones/:zone/servers")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_servers(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let parts = segments(&path);
        let (Some(project), Some(zone)) = (
            parts.get(1).map(|part| part.to_string()),
            parts.get(3).map(|part| part.to_string()),
        ) else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{project}/zones/{zone}");
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let mut servers: Vec<String> = form
            .keys()
            .filter_map(|key| key.strip_prefix("server-"))
            .map(str::to_string)
            .collect();
        servers.sort();
        let asked = crate::features::dns::set_zone_servers_request::SetZoneServersRequest {
            project,
            zone,
            servers,
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&here, &reason.to_string()));
        }
        match crate::features::dns::node_user_set_zone_servers::run(
            &self.state.database,
            &self.state.container,
            &account,
            &asked,
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::values::record_data::{RecordData, RecordKind};
    use crate::domain::values::record_name::RecordName;
    use crate::domain::values::zone_name::ZoneName;
    use crate::infra::repositories::zones;
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    async fn received(console: &Console, name: &str) {
        zones::replace_received(
            &console.database,
            zones::Received {
                name: &ZoneName::parse(name).expect("zone"),
                authority: "nd-far",
                serial: 1,
                default_ttl: 300,
                records: vec![(
                    RecordName::apex(),
                    RecordData::parse(RecordKind::A, "203.0.113.7", None).expect("data"),
                    300,
                )],
                name_servers: &["node-9.example.net".to_string()],
            },
        )
        .await
        .expect("a zone another node sent");
    }

    /// The node's DNS is a tab of its settings now, and what is on it is
    /// what no project holds: a zone another node sent is listed there, and
    /// put down from there.
    #[tokio::test]
    async fn a_zone_another_node_sent_is_listed_on_the_dns_tab_and_put_down_there() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        received(&console, "far.example").await;

        let tab = console
            .harness
            .get("/node/dns")
            .header("cookie", &admin)
            .send()
            .await;
        tab.assert_ok();
        assert!(tab.body.contains("far.example"), "the zone is not listed");
        assert!(tab.body.contains("action=\"/node/dns/far.example/delete\""));
        assert!(tab.body.contains("action=\"/node/dns/names\""));

        let put_down = console
            .harness
            .post("/node/dns/far.example/delete")
            .header("cookie", &admin)
            .send()
            .await;
        assert_eq!(put_down.header("location"), Some("/node/dns"));
        assert!(zones::find_by_name(&console.database, "far.example")
            .await
            .expect("read")
            .is_none());
    }

    /// A project's zone is deleted by the project's people, from the
    /// project; from the machine's tab it reads as one that is not there.
    #[tokio::test]
    async fn a_projects_zone_is_not_put_down_from_the_dns_tab() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &admin)
            .form(&[("name", "Site")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        console
            .harness
            .post("/projects/site/zones")
            .header("cookie", &admin)
            .form(&[("name", "teresaasprilla.com")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let tab = console
            .harness
            .get("/node/dns")
            .header("cookie", &admin)
            .send()
            .await;
        assert!(!tab.body.contains("teresaasprilla.com"));

        let refused = console
            .harness
            .post("/node/dns/teresaasprilla.com/delete")
            .header("cookie", &admin)
            .send()
            .await;
        let location = refused.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/node/dns?error=there+is+no+such+zone"),
            "{location}"
        );
        assert!(zones::find_by_name(&console.database, "teresaasprilla.com")
            .await
            .expect("read")
            .is_some());
    }

    #[tokio::test]
    async fn only_the_machines_operator_puts_a_zone_down() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;
        received(&console, "far.example").await;

        let tab = console
            .harness
            .get("/node/dns")
            .header("cookie", &member)
            .send()
            .await;
        assert_eq!(tab.header("location"), Some("/"), "a member saw the tab");
        console
            .harness
            .post("/node/dns/far.example/delete")
            .header("cookie", &member)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        assert!(zones::find_by_name(&console.database, "far.example")
            .await
            .expect("read")
            .is_some());
    }
}
