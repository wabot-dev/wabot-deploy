//! Go to a thing by its name.
//!
//! The field is in the frame's header, on every page. A name that is one
//! thing's goes straight to it; anything else is a page listing what it
//! could have meant. A form and a GET, so it works with scripting off
//! and a search can be linked.
//!
//! Nobody is shown a name they could not have opened: the things come
//! from the same lists the pages draw, narrowed by the same question.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::shell::auth::{signed_in, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Frame, Section};
use crate::shell::state::ConsoleState;
use crate::web::console::go_page::{Hit, Kind, Results};

#[derive(Debug, Deserialize, Validate, Default)]
pub struct GoPage {
    pub q: Option<String>,
}

#[injectable]
pub struct GoUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

/// What a query names, best first: the things called exactly that, and
/// only when there are none, the things whose name contains it.
///
/// Exact first because the names nest — `orders` and `orders-worker` —
/// and somebody who typed the whole of one meant that one.
pub fn found(query: &str, things: &[Hit]) -> Vec<Hit> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let named = |hit: &&Hit, test: &dyn Fn(&str) -> bool| {
        test(&hit.label.to_lowercase()) || test(&hit.key.to_lowercase())
    };
    let exact: Vec<Hit> = things
        .iter()
        .filter(|hit| named(hit, &|name| name == query))
        .cloned()
        .collect();
    match exact.is_empty() {
        false => exact,
        true => things
            .iter()
            .filter(|hit| named(hit, &|name| name.contains(&query)))
            .cloned()
            .collect(),
    }
}

#[ui_controller("/", app)]
impl GoUiController {
    #[view("/go")]
    #[middleware(SessionMiddleware)]
    async fn go(&self, query: GoPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let asked = query.q.unwrap_or_default();

        let mut things = Vec::new();
        let projects =
            crate::infra::repositories::access::projects_for(&self.state.database, &account)
                .await?;
        let services =
            crate::infra::repositories::services::all(&self.state.database, None).await?;
        for project in &projects {
            let href = format!("/projects/{}", project.slug);
            things.push(Hit::new(Kind::Project, &project.name, &project.slug, &href));
            for service in services.iter().filter(|s| s.project_id == project.id) {
                things.push(
                    Hit::new(
                        Kind::Service,
                        &service.name,
                        &service.slug,
                        format!("{href}/services/{}", service.slug),
                    )
                    .inside(&project.name),
                );
            }
        }
        if crate::domain::authz::node_user::manages_the_machine(&account) {
            for node in crate::infra::network::all(&self.state.database).await? {
                things.push(Hit::new(
                    Kind::Node,
                    &node.name,
                    &node.id,
                    format!("/nodes/{}", node.id),
                ));
            }
        }
        if crate::domain::authz::node_user::shares_the_machine_out(&account) {
            for tenant in crate::infra::repositories::tenants::all(&self.state.database).await? {
                things.push(Hit::new(
                    Kind::Organisation,
                    &tenant.name,
                    &tenant.slug,
                    format!("/organisations/{}", tenant.id),
                ));
            }
        }

        let hits = found(&asked, &things);
        if let [only] = hits.as_slice() {
            return Ok(Redirect::found(only.href.clone()).into());
        }

        layout::head("Go to");
        let frame = Frame::load(&self.state.database, &account, Section::Search, "/go").await;
        let page = Results {
            asked: asked.trim(),
            hits: &hits,
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::web::console::go_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn things() -> Vec<Hit> {
        vec![
            Hit::new(Kind::Project, "Shop", "shop", "/projects/shop"),
            Hit::new(
                Kind::Service,
                "orders",
                "orders",
                "/projects/shop/services/orders",
            ),
            Hit::new(
                Kind::Service,
                "orders-worker",
                "orders-worker",
                "/projects/shop/services/orders-worker",
            ),
            Hit::new(Kind::Node, "node-l1", "nd-abc123", "/nodes/nd-abc123"),
        ]
    }

    /// The whole of a name means that thing, though another name
    /// contains it. Otherwise `orders` could never be reached by typing
    /// it while `orders-worker` exists.
    #[test]
    fn a_whole_name_is_that_thing_and_no_other() {
        let hits = found("orders", &things());
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].href, "/projects/shop/services/orders");
    }

    #[test]
    fn part_of_a_name_offers_everything_it_could_be() {
        let hits = found("ord", &things());
        assert_eq!(hits.len(), 2, "{hits:?}");
    }

    /// Case is not part of a name somebody is typing from memory, and an
    /// id finds its node as well as the name does.
    #[test]
    fn a_name_is_found_whatever_its_case_and_a_node_by_its_id() {
        assert_eq!(found("SHOP", &things())[0].href, "/projects/shop");
        assert_eq!(found("nd-abc123", &things())[0].href, "/nodes/nd-abc123");
        assert_eq!(found(" node-L1 ", &things())[0].href, "/nodes/nd-abc123");
    }

    /// A name goes straight to the thing, and nobody is taken to — or
    /// shown — something they could not have opened.
    ///
    /// The second half is the one that matters: a search over everything
    /// narrowed afterwards is how a member learns what projects there
    /// are by typing letters.
    #[tokio::test]
    async fn a_name_goes_to_the_thing_and_only_for_who_can_open_it() {
        use crate::web::console::tests::Console;
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;
        console
            .harness
            .post("/projects")
            .header("cookie", &admin)
            .form(&[("name", "Shop")])
            .send()
            .await;
        let project = crate::infra::repositories::projects::all(&console.database)
            .await
            .expect("projects")
            .pop()
            .expect("made");
        crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "orders",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        let node_id = console.node_path.trim_start_matches("/nodes/").to_string();
        macro_rules! go {
            ($cookie:expr, $q:expr) => {
                console
                    .harness
                    .get(&format!("/go?q={}", $q))
                    .header("cookie", $cookie)
                    .send()
                    .await
            };
        }

        assert_eq!(
            go!(&admin, "orders").header("location"),
            Some("/projects/shop/services/orders")
        );
        assert_eq!(
            go!(&admin, "SHOP").header("location"),
            Some("/projects/shop")
        );
        assert_eq!(
            go!(&admin, &node_id).header("location"),
            Some(console.node_path.as_str())
        );

        for name in ["shop", "orders", node_id.as_str()] {
            let answer = go!(&member, name);
            assert_eq!(
                answer.header("location"),
                None,
                "a member was taken to {name}"
            );
            assert!(
                answer.body.contains("Nothing here is called that."),
                "a member was told {name} exists: {}",
                answer.body
            );
        }

        assert_eq!(
            console
                .harness
                .get("/go?q=shop")
                .send()
                .await
                .header("location"),
            Some("/sign-in")
        );
    }

    #[test]
    fn nothing_typed_finds_nothing() {
        assert!(found("", &things()).is_empty());
        assert!(found("   ", &things()).is_empty());
        assert!(found("no-such-thing", &things()).is_empty());
    }
}
