//! What Your account posts to. Each door reads the form, asks again where
//! the action makes a way in, and hands the use case of whichever console
//! the account belongs to.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::features::account::add_device_request::AddDeviceRequest;
use crate::features::account::add_passkey_request::AddPasskeyRequest;
use crate::features::account::answer_device_request::AnswerDeviceRequest;
use crate::features::account::remove_passkey_request::RemovePasskeyRequest;
use crate::features::account::rename_passkey_request::RenamePasskeyRequest;
use crate::features::account::withdraw_device_request::WithdrawDeviceRequest;
use crate::features::account::{
    node_user_add_a_device, node_user_add_a_passkey, node_user_approve_a_device,
    node_user_refuse_a_device, node_user_remove_a_passkey, node_user_rename_a_passkey,
    node_user_withdraw_a_device, tenant_user_add_a_device, tenant_user_add_a_passkey,
    tenant_user_approve_a_device, tenant_user_refuse_a_device, tenant_user_remove_a_passkey,
    tenant_user_rename_a_passkey, tenant_user_withdraw_a_device,
};
use crate::infra::accounts::{devices, sessions};
use crate::shell::auth::*;
use crate::shell::state::ConsoleState;

#[injectable]
pub struct AccountRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

fn nth(request: &Request, index: usize) -> String {
    segments(request.uri().path())
        .get(index)
        .map(|segment| segment.to_string())
        .unwrap_or_default()
}

#[rest_controller("/")]
impl AccountRestController {
    /// What the header's icon says, sent again whenever it changes: a
    /// device asking on every page somebody has open, and gone from all of
    /// them once one of them answers.
    ///
    /// The session is asked again on every turn, so signing out everywhere
    /// also stops a stream that was already open.
    #[get("/account/live")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn live(&self, request: Request) -> RestResult<Response> {
        use crate::shell::components::account_control::{AccountControl, Glance};
        use wabot::rest::axum::body::{Body, Bytes};
        use wabot::rest::axum::http::{header, StatusCode};

        let (Some(account), Some(token)) = (
            signed_in(&self.auth),
            sessions::from_headers(request.headers()),
        ) else {
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        };
        let database = self.state.database.clone();
        let stream = async_stream::stream! {
            let mut shown = String::new();
            let mut quiet = 0u32;
            loop {
                if !matches!(sessions::find(&database, &token).await, Ok(Some(_))) {
                    break;
                }
                let glance = Glance::read(&database, &account.id).await;
                if glance.key() != shown {
                    let control = AccountControl::load(&database, &account, false).await;
                    shown = control.glance.key();
                    let html = crate::shell::texts::language::scoped(account.language, || {
                        control.inner()
                    });
                    let payload = serde_json::json!({
                        "key": shown,
                        "asking": control.glance.request.as_ref().map(|request| request.id.clone()),
                        "html": html,
                    });
                    quiet = 0;
                    yield Ok::<_, std::convert::Infallible>(Bytes::from(format!("data: {payload}\n\n")));
                } else {
                    quiet += 1;
                    if quiet.is_multiple_of(10) {
                        yield Ok(Bytes::from_static(b": still here\n\n"));
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        };
        Ok(Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .expect("a constant response is well-formed"))
    }

    #[post("/account/passkeys")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_a_passkey(&self, request: Request) -> RestResult<Response> {
        let (Some(account), Some(held)) = (signed_in(&self.auth), session(&self.auth)) else {
            return Ok(see_other("/sign-in"));
        };
        let browser = browser(&request);
        let cookie = sessions::from_headers(request.headers());
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        if let Err(reason) = proven(&self.state, &self.auth, &browser, &form).await {
            return Ok(back_with_error("/account", &reason));
        }
        let asked = AddPasskeyRequest {
            passkey: ceremony(&form, ""),
            browser,
            session: held.token_hash,
        };
        let added = match account.tenant_id {
            Some(_) => tenant_user_add_a_passkey::run(&self.state.database, &account, &asked).await,
            None => node_user_add_a_passkey::run(&self.state.database, &account, &asked).await,
        };
        match added {
            Ok(added) => {
                let mut cookies = Vec::new();
                if let Some(device) = &added.device {
                    cookies.push(devices::set_cookie(device));
                    if let Some(token) = &cookie {
                        cookies.push(sessions::set_cookie(token, true));
                    }
                }
                Ok(see_other_with_cookies("/account", &cookies))
            }
            Err(reason) => Ok(back_with_error("/account", &reason)),
        }
    }

    #[post("/account/passkeys/:id/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove_a_passkey(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let (browser, passkey_id) = (browser(&request), nth(&request, 2));
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        if let Err(reason) = proven(&self.state, &self.auth, &browser, &form).await {
            return Ok(back_with_error("/account", &reason));
        }
        let asked = RemovePasskeyRequest {
            passkey_id,
            address: browser.address,
        };
        let removed = match account.tenant_id {
            Some(_) => {
                tenant_user_remove_a_passkey::run(&self.state.database, &account, &asked).await
            }
            None => node_user_remove_a_passkey::run(&self.state.database, &account, &asked).await,
        };
        match removed {
            Ok(()) => Ok(see_other("/account")),
            Err(reason) => Ok(back_with_error("/account", &reason)),
        }
    }

    #[post("/account/passkeys/:id/rename")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn rename_a_passkey(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let passkey_id = nth(&request, 2);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = RenamePasskeyRequest {
            passkey_id,
            label: field(&form, "label").to_string(),
        };
        let renamed = match account.tenant_id {
            Some(_) => {
                tenant_user_rename_a_passkey::run(&self.state.database, &account, &asked).await
            }
            None => node_user_rename_a_passkey::run(&self.state.database, &account, &asked).await,
        };
        match renamed {
            Ok(()) => Ok(see_other("/account")),
            Err(reason) => Ok(back_with_error("/account", &reason)),
        }
    }

    #[post("/account/devices/:id/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn withdraw_a_device(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let this_one = session(&self.auth).and_then(|held| held.device_id);
        let asked = WithdrawDeviceRequest {
            device_id: nth(&request, 2),
            address: browser(&request).address,
        };
        let withdrawn = match account.tenant_id {
            Some(_) => {
                tenant_user_withdraw_a_device::run(&self.state.database, &account, &asked).await
            }
            None => node_user_withdraw_a_device::run(&self.state.database, &account, &asked).await,
        };
        match withdrawn {
            Ok(()) if this_one.as_deref() == Some(asked.device_id.as_str()) => {
                Ok(see_other("/sign-in"))
            }
            Ok(()) => Ok(see_other("/account/devices")),
            Err(reason) => Ok(back_with_error("/account/devices", &reason)),
        }
    }

    #[post("/account/devices/add")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_a_device(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let browser = browser(&request);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        if let Err(reason) = proven(&self.state, &self.auth, &browser, &form).await {
            return Ok(back_with_error("/account/devices", &reason));
        }
        let asked = AddDeviceRequest {
            trusted: session(&self.auth).is_some_and(|held| held.trusted()),
        };
        let link = match account.tenant_id {
            Some(_) => tenant_user_add_a_device::run(&self.state.database, &account, &asked).await,
            None => node_user_add_a_device::run(&self.state.database, &account, &asked).await,
        };
        match link {
            Ok(link) => {
                let url = format!("https://{}/link/{}", browser.host, link.token);
                let nonce = self
                    .state
                    .reveals
                    .stash(format!("{url}\n{}", link.expires_at));
                Ok(see_other(&format!(
                    "/account/added?{}",
                    encode("nonce", &nonce)
                )))
            }
            Err(reason) => Ok(back_with_error("/account/devices", &reason)),
        }
    }

    #[post("/account/requests/:id/approve")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn approve_a_device(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let (browser, request_id) = (browser(&request), nth(&request, 2));
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        if let Err(reason) = proven(&self.state, &self.auth, &browser, &form).await {
            return Ok(back_with_error("/account/approve", &reason));
        }
        let asked = AnswerDeviceRequest {
            request_id,
            trusted: session(&self.auth).is_some_and(|held| held.trusted()),
            address: browser.address,
        };
        let approved = match account.tenant_id {
            Some(_) => {
                tenant_user_approve_a_device::run(&self.state.database, &account, &asked).await
            }
            None => node_user_approve_a_device::run(&self.state.database, &account, &asked).await,
        };
        match approved {
            Ok(approved) => {
                let nonce = self.state.reveals.stash(format!(
                    "{}\n{}\n{}",
                    approved.code, approved.name, approved.expires_at
                ));
                Ok(see_other(&format!(
                    "/account/code?{}",
                    encode("nonce", &nonce)
                )))
            }
            Err(reason) => Ok(back_with_error("/account/approve", &reason)),
        }
    }

    #[post("/account/requests/:id/refuse")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn refuse_a_device(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let asked = AnswerDeviceRequest {
            request_id: nth(&request, 2),
            trusted: session(&self.auth).is_some_and(|held| held.trusted()),
            address: browser(&request).address,
        };
        let refused = match account.tenant_id {
            Some(_) => {
                tenant_user_refuse_a_device::run(&self.state.database, &account, &asked).await
            }
            None => node_user_refuse_a_device::run(&self.state.database, &account, &asked).await,
        };
        match refused {
            Ok(refused) => Ok(see_other(&format!(
                "/account?{}",
                encode("refused", &refused.passkey_id)
            ))),
            Err(reason) => Ok(back_with_error("/account", &reason)),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::{header, StatusCode};
    use wabot::testing::TestResponse;
    use wabot_addon_auth_passkey::testing::SoftAuthenticator;
    use wabot_addon_auth_passkey::RelyingParty;

    pub(crate) const HOST: &str = "node.example.com";

    pub(crate) fn rp() -> RelyingParty {
        crate::infra::accounts::ceremonies::relying_party(HOST).expect("a name")
    }

    /// What a response set, as the `name=value` a browser sends back.
    pub(crate) fn set(response: &TestResponse, name: &str) -> Option<String> {
        response
            .headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with(&format!("{name}=")))
            .map(str::to_string)
    }

    pub(crate) fn pair(cookie: &str) -> String {
        cookie.split(';').next().unwrap_or_default().to_string()
    }

    pub(crate) fn field_on(body: &str, name: &str) -> String {
        let marker = format!("name=\"{name}\" value=\"");
        let (_, rest) = body
            .split_once(&marker)
            .unwrap_or_else(|| panic!("no {name} on the page: {body}"));
        rest.split('"').next().unwrap_or_default().to_string()
    }

    pub(crate) async fn handle_for(console: &Console, challenge: &str) -> String {
        let challenge = challenge.to_string();
        console
            .database
            .read(move |connection| {
                connection.query_row(
                    "SELECT \"user_handle\" FROM passkey_challenge WHERE \"challenge\" = ?1",
                    [challenge],
                    |row| row.get::<_, String>(0),
                )
            })
            .await
            .expect("the challenge the page rendered")
    }

    pub(crate) async fn page(console: &Console, path: &str, cookie: &str) -> TestResponse {
        console
            .harness
            .get(path)
            .header("host", HOST)
            .header("cookie", cookie)
            .send()
            .await
    }

    pub(crate) async fn post(
        console: &Console,
        path: &str,
        cookie: &str,
        fields: &[(&str, &str)],
    ) -> TestResponse {
        console
            .harness
            .post(path)
            .header("host", HOST)
            .header("cookie", cookie)
            .form(fields)
            .send()
            .await
    }

    /// Claim the node with a passkey, as its operator's browser would:
    /// their device, and the session and device cookies it now holds.
    pub(crate) async fn claimed(console: &Console) -> (SoftAuthenticator, String, String) {
        let body = page(console, "/setup", "").await.body;
        let challenge = field_on(&body, "challenge");
        let handle = handle_for(console, &challenge).await;
        let device = SoftAuthenticator::new();
        let credential = device.register(&rp(), &challenge, handle.as_bytes());

        let response = post(
            console,
            "/setup",
            "",
            &[
                ("setup_token", &console.setup_token),
                ("username", "jorge"),
                ("challenge", &challenge),
                ("credential", &credential),
            ],
        )
        .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let session =
            set(&response, crate::infra::accounts::sessions::COOKIE_NAME).expect("signed in");
        assert!(
            session.contains("Max-Age="),
            "a trusted device's session lasts: {session}"
        );
        let trusted = set(&response, crate::infra::accounts::devices::COOKIE_NAME)
            .expect("the setup token trusted this browser");
        (device, pair(&session), pair(&trusted))
    }

    async fn signed_in_with(
        console: &Console,
        device: &SoftAuthenticator,
        cookie: &str,
    ) -> TestResponse {
        let body = page(console, "/sign-in", cookie).await.body;
        let challenge = field_on(&body, "challenge");
        let credential = device.authenticate(&rp(), &challenge);
        post(
            console,
            "/sign-in/passkey",
            cookie,
            &[("challenge", &challenge), ("credential", &credential)],
        )
        .await
    }

    #[tokio::test]
    async fn on_the_browser_setup_trusted_the_passkey_alone_signs_in() {
        let console = Console::new().await;
        let (device, _, trusted) = claimed(&console).await;

        let response = signed_in_with(&console, &device, &trusted).await;

        assert_eq!(response.header("location"), Some("/"));
        let session =
            set(&response, crate::infra::accounts::sessions::COOKIE_NAME).expect("signed in");
        assert!(session.contains("Max-Age="), "{session}");
    }

    /// The passkey syncs, so on a browser nobody trusts it opens a request
    /// a trusted device has to answer; the code it shows is typed there.
    #[tokio::test]
    async fn a_new_device_waits_for_a_trusted_one_and_types_the_code_it_shows() {
        let console = Console::new().await;
        let (device, operator, _) = claimed(&console).await;

        let asked = signed_in_with(&console, &device, "").await;
        assert_eq!(asked.header("location"), Some("/sign-in/approval"));
        let pending =
            pair(&set(&asked, crate::infra::accounts::requests::COOKIE_NAME).expect("a request"));
        assert!(set(&asked, crate::infra::accounts::sessions::COOKIE_NAME).is_none());

        let early = post(
            &console,
            "/sign-in/approval",
            &pending,
            &[("code", "123456")],
        )
        .await;
        assert!(
            early
                .header("location")
                .is_some_and(|to| to.contains("approved")),
            "{:?}",
            early.header("location")
        );

        let elsewhere = page(&console, "/projects", &operator).await.body;
        assert!(elsewhere.contains("asks to sign in as you"), "{elsewhere}");
        let approve = page(&console, "/account/approve", &operator).await.body;
        let id = approve
            .split("/account/requests/")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .expect("the request on the page")
            .to_string();

        let approved = post(
            &console,
            &format!("/account/requests/{id}/approve"),
            &operator,
            &[],
        )
        .await;
        let shown = approved.header("location").expect("the code").to_string();
        assert!(shown.starts_with("/account/code?"), "{shown}");
        let body = page(&console, &shown, &operator).await.body;
        let code = body
            .split("<p class=\"code-big\">")
            .nth(1)
            .and_then(|rest| rest.split('<').next())
            .expect("six digits")
            .trim()
            .to_string();
        assert_eq!(code.len(), 7, "two threes, as they are read: {code}");
        assert_eq!(code.matches(char::is_numeric).count(), 6, "{code}");

        let joined = post(
            &console,
            "/sign-in/approval",
            &pending,
            &[("code", &code), ("trust", "on")],
        )
        .await;
        assert_eq!(joined.header("location"), Some("/"));
        assert!(set(&joined, crate::infra::accounts::sessions::COOKIE_NAME).is_some());
        assert!(set(&joined, crate::infra::accounts::devices::COOKIE_NAME).is_some());

        let activity = page(&console, "/account/activity", &operator).await.body;
        assert!(activity.contains("Sign-in approved"), "{activity}");
        assert!(activity.contains("Device trusted"), "{activity}");
    }

    #[tokio::test]
    async fn it_was_not_me_closes_the_request_and_names_the_passkey_it_used() {
        let console = Console::new().await;
        let (device, operator, _) = claimed(&console).await;
        let asked = signed_in_with(&console, &device, "").await;
        let pending =
            pair(&set(&asked, crate::infra::accounts::requests::COOKIE_NAME).expect("a request"));
        let approve = page(&console, "/account/approve", &operator).await.body;
        let id = approve
            .split("/account/requests/")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .expect("the request")
            .to_string();

        let refused = post(
            &console,
            &format!("/account/requests/{id}/refuse"),
            &operator,
            &[],
        )
        .await;
        let to = refused.header("location").expect("back").to_string();
        assert!(to.starts_with("/account?refused="), "{to}");
        let body = page(&console, &to, &operator).await.body;
        assert!(body.contains("Somebody else has it"), "{body}");

        let late = post(
            &console,
            "/sign-in/approval",
            &pending,
            &[("code", "000000")],
        )
        .await;
        assert!(
            late.header("location")
                .is_some_and(|to| to.contains("closed")),
            "{:?}",
            late.header("location")
        );
    }

    /// A stolen session cookie must not become permanent access: past five
    /// minutes, handing out a way in asks for the passkey again.
    #[tokio::test]
    async fn adding_a_device_asks_for_the_passkey_again_and_the_link_trusts_the_phone() {
        let console = Console::new().await;
        let (device, operator, _) = claimed(&console).await;
        console
            .database
            .write(|connection| {
                connection.execute("UPDATE session SET \"verified_at\" = 1", [])?;
                Ok(())
            })
            .await
            .expect("aged");

        let refused = post(&console, "/account/devices/add", &operator, &[]).await;
        assert!(
            refused
                .header("location")
                .is_some_and(|to| to.contains("passkey+again")),
            "{:?}",
            refused.header("location")
        );

        let body = page(&console, "/account/devices", &operator).await.body;
        let challenge = field_on(&body, "verify_challenge");
        let answer = device.authenticate(&rp(), &challenge);
        let added = post(
            &console,
            "/account/devices/add",
            &operator,
            &[
                ("verify_challenge", &challenge),
                ("verify_credential", &answer),
            ],
        )
        .await;
        let shown = added.header("location").expect("the link").to_string();
        assert!(shown.starts_with("/account/added?"), "{shown}");
        let link = console.secret_at(&shown, &operator).await;
        let token = link.rsplit('/').next().expect("a token").to_string();
        assert!(link.starts_with(&format!("https://{HOST}/link/")), "{link}");

        let opened = page(&console, &format!("/link/{token}"), "").await.body;
        let challenge = field_on(&opened, "challenge");
        let handle = handle_for(&console, &challenge).await;
        let phone = SoftAuthenticator::new();
        let made = phone.register(&rp(), &challenge, handle.as_bytes());
        let used = post(
            &console,
            &format!("/link/{token}"),
            "",
            &[("challenge", &challenge), ("credential", &made)],
        )
        .await;
        assert_eq!(used.header("location"), Some("/"));
        let phone_device =
            pair(&set(&used, crate::infra::accounts::devices::COOKIE_NAME).expect("trusted"));

        let again = signed_in_with(&console, &phone, &phone_device).await;
        assert_eq!(again.header("location"), Some("/"));

        let spent = post(&console, &format!("/link/{token}"), "", &[]).await;
        assert!(spent
            .header("location")
            .is_some_and(|to| to.contains("not+valid")));
    }

    /// An account from before passkeys keeps its password until it makes
    /// one, and the first one trusts the browser it was made on.
    #[tokio::test]
    async fn the_first_passkey_deletes_the_password_and_trusts_the_browser() {
        let console = Console::new().await;
        console.signed_in().await;
        let jorge = crate::infra::accounts::by_username(&console.database, "jorge", None)
            .await
            .expect("read")
            .expect("jorge");
        crate::infra::accounts::legacy_password(&console.database, &jorge.id, "an old password")
            .await;

        let signed = post(
            &console,
            "/sign-in",
            "",
            &[("username", "jorge"), ("password", "an old password")],
        )
        .await;
        let session = set(&signed, crate::infra::accounts::sessions::COOKIE_NAME).expect("in");
        assert!(
            !session.contains("Max-Age="),
            "no device is trusted: {session}"
        );
        let session = pair(&session);

        let body = page(&console, "/account", &session).await.body;
        assert!(
            body.contains("Your first passkey deletes your password"),
            "{body}"
        );
        let challenge = field_on(&body, "challenge");
        let device = SoftAuthenticator::new();
        let made = device.register(&rp(), &challenge, jorge.id.as_bytes());
        let added = post(
            &console,
            "/account/passkeys",
            &session,
            &[("challenge", &challenge), ("credential", &made)],
        )
        .await;
        assert_eq!(added.header("location"), Some("/account"));
        assert!(set(&added, crate::infra::accounts::devices::COOKIE_NAME).is_some());
        assert!(set(&added, crate::infra::accounts::sessions::COOKIE_NAME)
            .is_some_and(|cookie| cookie.contains("Max-Age=")));

        assert!(
            !crate::infra::accounts::has_password(&console.database, &jorge.id)
                .await
                .expect("read")
        );
        let page = page(&console, "/sign-in", "").await.body;
        assert!(!page.contains("name=\"password\""), "{page}");
    }

    /// A name its owner gave it, and back to where it lives when the
    /// field is emptied. Nobody renames another account's passkey.
    #[tokio::test]
    async fn a_passkey_is_called_what_its_owner_names_it() {
        let console = Console::new().await;
        let (_, operator, _) = claimed(&console).await;
        let jorge = crate::infra::accounts::named(&console.database, "jorge")
            .await
            .expect("read")
            .remove(0);
        let id = crate::infra::accounts::passkeys::of_account(&console.database, &jorge.id)
            .await
            .expect("read")
            .remove(0)
            .id;

        let renamed = post(
            &console,
            &format!("/account/passkeys/{id}/rename"),
            &operator,
            &[("label", "  Work   laptop ")],
        )
        .await;
        assert_eq!(renamed.header("location"), Some("/account"));
        let body = page(&console, "/account", &operator).await.body;
        assert!(body.contains("Work laptop"), "{body}");

        post(
            &console,
            &format!("/account/passkeys/{id}/rename"),
            &operator,
            &[("label", "")],
        )
        .await;
        let passkey = crate::infra::accounts::passkeys::find(&console.database, &id)
            .await
            .expect("read")
            .expect("still there");
        assert_eq!(passkey.label, None);

        let refused = post(
            &console,
            "/account/passkeys/somebody-elses/rename",
            &operator,
            &[("label", "mine now")],
        )
        .await;
        assert!(
            refused
                .header("location")
                .is_some_and(|to| to.starts_with("/account?error=")),
            "{:?}",
            refused.header("location")
        );
    }

    /// The header's icon follows a stream, and a stranger gets a status
    /// rather than a redirect an `EventSource` cannot follow.
    #[tokio::test]
    async fn the_header_s_stream_answers_only_a_session() {
        let console = Console::new().await;
        claimed(&console).await;

        let stranger = page(&console, "/account/live", "").await;

        stranger.assert_status(StatusCode::UNAUTHORIZED);
    }

    /// The field offers this host's passkeys from the browser's autofill,
    /// and while a password exists here it is the password's field too,
    /// so one field serves both ways in.
    #[tokio::test]
    async fn the_username_field_offers_passkeys_and_the_password_folds_under_them() {
        let console = Console::new().await;
        claimed(&console).await;

        let passkeys_only = page(&console, "/sign-in", "").await.body;
        assert!(
            passkeys_only.contains("autocomplete=\"username webauthn\""),
            "{passkeys_only}"
        );
        assert!(
            passkeys_only.contains("autofill&quot;:true")
                || passkeys_only.contains("\"autofill\":true"),
            "{passkeys_only}"
        );
        assert!(!passkeys_only.contains("password-form"), "{passkeys_only}");

        crate::infra::accounts::legacy_password(
            &console.database,
            &crate::infra::accounts::named(&console.database, "jorge")
                .await
                .expect("read")
                .remove(0)
                .id,
            "an old password",
        )
        .await;
        let both = page(&console, "/sign-in", "").await.body;
        assert!(both.contains("form=\"password-form\""), "{both}");
        assert!(both.contains("<details class=\"other-way\">"), "{both}");
    }
}
