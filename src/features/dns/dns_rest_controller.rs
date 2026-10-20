use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::shell::auth::{
    back_with_error, read_form, see_other, segments, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct DnsRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl DnsRestController {
    #[post("/dns/zones")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create_zone(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = crate::features::dns::create_zone_request::CreateZoneRequest {
            name: form.get("name").cloned().unwrap_or_default(),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error("/dns", &reason.to_string()));
        }
        match crate::features::dns::node_user_create_zone::run(
            &self.state.database,
            &self.state.container,
            &account,
            &asked,
        )
        .await
        {
            Ok(created) => Ok(see_other(&format!("/dns/{}", created.name))),
            Err(reason) => Ok(back_with_error("/dns", &reason)),
        }
    }

    #[post("/dns/:zone/records")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_record(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(zone) = segments(&path).get(1).map(|zone| zone.to_string()) else {
            return Ok(see_other("/dns"));
        };
        let here = format!("/dns/{zone}");
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let field = |name: &str| form.get(name).cloned().unwrap_or_default();
        let asked = crate::features::dns::add_record_request::AddRecordRequest {
            zone,
            name: field("name"),
            kind: field("kind"),
            value: field("value"),
            priority: field("priority"),
            ttl: field("ttl"),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&here, &reason.to_string()));
        }
        match crate::features::dns::node_user_add_record::run(
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

    #[post("/dns/:zone/servers")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_servers(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(zone) = segments(&path).get(1).map(|zone| zone.to_string()) else {
            return Ok(see_other("/dns"));
        };
        let here = format!("/dns/{zone}");
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
        let asked =
            crate::features::dns::set_zone_servers_request::SetZoneServersRequest { zone, servers };
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

    #[post("/dns/:zone/records/:record/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove_record(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let parts = segments(&path);
        let (Some(zone), Some(record)) = (parts.get(1), parts.get(3)) else {
            return Ok(see_other("/dns"));
        };
        let here = format!("/dns/{zone}");
        match crate::features::dns::node_user_remove_record::run(
            &self.state.database,
            &self.state.container,
            &account,
            zone,
            record,
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/dns/:zone/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete_zone(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(zone) = segments(&path).get(1).map(|zone| zone.to_string()) else {
            return Ok(see_other("/dns"));
        };
        match crate::features::dns::node_user_delete_zone::run(
            &self.state.database,
            &self.state.container,
            &account,
            &zone,
        )
        .await
        {
            Ok(()) => Ok(see_other("/dns")),
            Err(reason) => Ok(back_with_error(&format!("/dns/{zone}"), &reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::repositories::zones;
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    async fn add_zone(console: &Console, cookie: &str, name: &str) -> wabot::testing::TestResponse {
        console
            .harness
            .post("/dns/zones")
            .header("cookie", cookie)
            .form(&[("name", name)])
            .send()
            .await
    }

    async fn add_record(
        console: &Console,
        cookie: &str,
        zone: &str,
        fields: &[(&str, &str)],
    ) -> wabot::testing::TestResponse {
        let mut form = vec![("zone", zone)];
        form.extend_from_slice(fields);
        console
            .harness
            .post(&format!("/dns/{zone}/records"))
            .header("cookie", cookie)
            .form(&form)
            .send()
            .await
    }

    #[tokio::test]
    async fn only_an_administrator_of_the_node_reaches_dns() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        let page = console
            .harness
            .get("/dns")
            .header("cookie", &member)
            .send()
            .await;
        assert_eq!(
            page.header("location"),
            Some("/"),
            "a member was shown the zones"
        );

        let refused = add_zone(&console, &member, "teresaasprilla.com").await;
        refused.assert_status(StatusCode::SEE_OTHER);
        assert!(
            zones::all(&console.database)
                .await
                .expect("zones")
                .is_empty(),
            "a member created a zone"
        );
    }

    /// The page and the name server read the same rows: what the form
    /// saved is what a resolver is told.
    #[tokio::test]
    async fn a_zone_made_in_the_console_is_what_the_name_server_answers_from() {
        let console = Console::new().await;
        let admin = console.signed_in().await;

        let created = add_zone(&console, &admin, "TeresaAsprilla.com").await;
        assert_eq!(created.header("location"), Some("/dns/teresaasprilla.com"));
        let added = add_record(
            &console,
            &admin,
            "teresaasprilla.com",
            &[
                ("name", "@"),
                ("kind", "A"),
                ("value", "209.50.48.176"),
                ("ttl", ""),
            ],
        )
        .await;
        assert_eq!(added.header("location"), Some("/dns/teresaasprilla.com"));

        let page = console
            .harness
            .get("/dns/teresaasprilla.com")
            .header("cookie", &admin)
            .send()
            .await;
        page.assert_ok();
        assert!(
            page.body.contains("209.50.48.176"),
            "the record is on its page"
        );

        let served = crate::infra::dns::what_this_node_serves(&console.database)
            .await
            .expect("served");
        assert_eq!(served.len(), 1);
        assert_eq!(served[0].zone.name.as_str(), "teresaasprilla.com");
        assert_eq!(served[0].records.len(), 1);
    }

    #[tokio::test]
    async fn a_refused_record_says_why_and_saves_nothing() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        add_zone(&console, &admin, "teresaasprilla.com").await;

        let refused = add_record(
            &console,
            &admin,
            "teresaasprilla.com",
            &[("name", "@"), ("kind", "A"), ("value", "not-an-address")],
        )
        .await;
        let location = refused.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/dns/teresaasprilla.com?error=an+A+record"),
            "the refusal goes back to the page it came from: {location}"
        );
        let zone = zones::find_by_name(&console.database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("zone");
        assert!(zones::records(&console.database, &zone)
            .await
            .expect("records")
            .is_empty());
    }

    #[tokio::test]
    async fn a_deleted_zone_is_answered_by_nobody() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        add_zone(&console, &admin, "teresaasprilla.com").await;
        add_record(
            &console,
            &admin,
            "teresaasprilla.com",
            &[
                ("name", "www"),
                ("kind", "CNAME"),
                ("value", "teresaasprilla.com"),
            ],
        )
        .await;
        let zone = zones::find_by_name(&console.database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("zone");
        let record = zones::records(&console.database, &zone)
            .await
            .expect("records")[0]
            .id
            .clone();

        console
            .harness
            .post(&format!("/dns/teresaasprilla.com/records/{record}/delete"))
            .header("cookie", &admin)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        assert!(zones::records(&console.database, &zone)
            .await
            .expect("records")
            .is_empty());

        let deleted = console
            .harness
            .post("/dns/teresaasprilla.com/delete")
            .header("cookie", &admin)
            .send()
            .await;
        assert_eq!(deleted.header("location"), Some("/dns"));
        assert!(crate::infra::dns::what_this_node_serves(&console.database)
            .await
            .expect("served")
            .is_empty());
    }

    /// The checkbox for a node that never agreed is disabled, which is a
    /// courtesy; this is the check that counts, on the POST.
    #[tokio::test]
    async fn only_a_node_that_agreed_can_be_chosen_to_answer() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        add_zone(&console, &admin, "teresaasprilla.com").await;
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-stranger".into(),
                name: "node-9.example.net".into(),
                kind: crate::infra::network::Kind::Public,
                endpoint: Some("node-9.example.net:443".into()),
                public_key: None,
                overlay_ip: Some("10.42.0.99".into()),
                is_self: false,
                last_seen_at: None,
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("a node that never agreed");
        let me = crate::infra::network::all(&console.database)
            .await
            .expect("nodes")
            .into_iter()
            .find(|node| node.is_self)
            .expect("this node")
            .id;

        let forged = console
            .harness
            .post("/dns/teresaasprilla.com/servers")
            .header("cookie", &admin)
            .form(&[
                (format!("server-{me}").as_str(), "1"),
                ("server-nd-stranger", "1"),
            ])
            .send()
            .await;
        let location = forged.header("location").unwrap_or_default();
        assert!(
            location.contains("error=that+node+has+not+agreed"),
            "{location}"
        );

        let zone = zones::find_by_name(&console.database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("zone");
        assert_eq!(
            zones::servers(&console.database, &zone.id)
                .await
                .expect("servers"),
            vec![me],
            "a refused choice changed nothing"
        );
    }
}
