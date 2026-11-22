//! Your account: the first page somebody has for themselves. It belongs
//! to the session, so both consoles serve it.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::domain::entities::account::Account;
use crate::infra::accounts::{self, ceremonies, devices, events, passkeys, requests};
use crate::shell::auth::{session, signed_in, verify_prompt, Asked, PageQuery, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Frame, Section};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct AccountUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
    asked: Arc<Asked>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct AccountQuery {
    pub error: Option<String>,
    pub refused: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ShownQuery {
    pub nonce: String,
}

impl AccountUiController {
    async fn framed(&self, account: &Account, path: &str, body: String) -> UiResult<ViewOutcome> {
        let frame = Frame::load(&self.state.database, account, Section::Account, path).await;
        Ok(frame.render(body).into_view().into())
    }

    fn rp_id(&self) -> String {
        ceremonies::relying_party(&self.asked.host)
            .map(|rp| rp.id)
            .unwrap_or_default()
    }
}

fn seconds_until(until: &str) -> i64 {
    until
        .parse::<i64>()
        .map(|at| (at - crate::infra::clock::now_ms()) / 1000)
        .unwrap_or(0)
}

#[ui_controller("/", app)]
impl AccountUiController {
    #[view("/account")]
    #[middleware(SessionMiddleware)]
    async fn passkeys(&self, query: AccountQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let database = &self.state.database;
        let mine = passkeys::of_account(database, &account.id).await?;
        let verify = verify_prompt(&self.state, &self.auth, &self.asked.host).await;
        let add = match ceremonies::relying_party(&self.asked.host) {
            Some(rp) => Some(
                ceremonies::begin_registration(
                    database,
                    &rp,
                    Some((&account.id, &account.username)),
                )
                .await?,
            ),
            None => None,
        };
        let refused = query
            .refused
            .as_deref()
            .and_then(|id| mine.iter().find(|passkey| passkey.id == id));
        let password = accounts::has_password(database, &account.id).await?;
        let host = self.rp_id();

        layout::head("Your account");
        let page = crate::features::account::account_page::PasskeysPage {
            passkeys: &mine,
            host: &host,
            add: add.as_ref(),
            verify: verify.as_ref(),
            username: &account.username,
            account_id: &account.id,
            password,
            error: query.error.as_deref(),
            refused,
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::account::account_page::render(&page)
                .render()
                .into_inner()
        });
        self.framed(&account, "/account", body).await
    }

    #[view("/account/devices")]
    #[middleware(SessionMiddleware)]
    async fn devices(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let held = session(&self.auth);
        let mine = devices::of_account(&self.state.database, &account.id).await?;
        let verify = verify_prompt(&self.state, &self.auth, &self.asked.host).await;
        let this_device = held.as_ref().and_then(|held| held.device_id.clone());

        layout::head("Your account");
        let page = crate::features::account::account_devices_page::DevicesPage {
            username: &account.username,
            devices: &mine,
            this_device: this_device.as_deref(),
            trusted: held.as_ref().is_some_and(|held| held.trusted()),
            verify: verify.as_ref(),
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::account::account_devices_page::render(&page)
                .render()
                .into_inner()
        });
        self.framed(&account, "/account/devices", body).await
    }

    #[view("/account/activity")]
    #[middleware(SessionMiddleware)]
    async fn activity(&self, _query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let happened = events::of_account(&self.state.database, &account.id, 200).await?;
        events::seen(&self.state.database, &account.id).await?;

        layout::head("Your account");
        let page = crate::features::account::account_activity_page::ActivityPage {
            username: &account.username,
            events: &happened,
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::account::account_activity_page::render(&page)
                .render()
                .into_inner()
        });
        self.framed(&account, "/account/activity", body).await
    }

    #[view("/account/approve")]
    #[middleware(SessionMiddleware)]
    async fn approve(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let pending = requests::pending_for(&self.state.database, &account.id).await?;
        let trusted = session(&self.auth).is_some_and(|held| held.trusted());
        let verify = match (&pending, trusted) {
            (Some(_), true) => verify_prompt(&self.state, &self.auth, &self.asked.host).await,
            _ => None,
        };
        let passkey = match &pending {
            Some(request) => passkeys::find(&self.state.database, &request.passkey_id)
                .await?
                .map(|passkey| passkey.title().to_string()),
            None => None,
        };

        layout::head("Your account");
        let page = crate::features::account::account_approve_page::ApprovePage {
            request: pending.as_ref(),
            passkey: passkey.as_deref(),
            trusted,
            verify: verify.as_ref(),
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::account::account_approve_page::render(&page)
                .render()
                .into_inner()
        });
        self.framed(&account, "/account/approve", body).await
    }

    #[view("/account/code")]
    #[middleware(SessionMiddleware)]
    async fn code(&self, query: ShownQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let taken = self.state.reveals.take(&query.nonce);
        let parts: Vec<&str> = taken.as_deref().unwrap_or("").splitn(3, '\n').collect();
        let shown = match parts.as_slice() {
            [code, name, until] if !code.is_empty() => {
                crate::features::account::account_shown_page::Shown::Code {
                    code,
                    name,
                    left: seconds_until(until),
                }
            }
            _ => crate::features::account::account_shown_page::Shown::Gone,
        };
        layout::head("Your account");
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::account::account_shown_page::render(&shown)
                .render()
                .into_inner()
        });
        self.framed(&account, "/account/devices", body).await
    }

    #[view("/account/added")]
    #[middleware(SessionMiddleware)]
    async fn added(&self, query: ShownQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let taken = self.state.reveals.take(&query.nonce);
        let (url, until) = taken
            .as_deref()
            .and_then(|text| text.split_once('\n'))
            .unwrap_or_default();
        let taken = Some(url.to_string()).filter(|url| !url.is_empty());
        let qr = taken.as_deref().and_then(|url| {
            qrcode::QrCode::new(url.as_bytes()).ok().map(|code| {
                code.render::<qrcode::render::svg::Color>()
                    .min_dimensions(200, 200)
                    .build()
            })
        });
        let shown = match (&taken, &qr) {
            (Some(url), Some(qr)) => crate::features::account::account_shown_page::Shown::Link {
                url,
                qr,
                left: seconds_until(until),
            },
            _ => crate::features::account::account_shown_page::Shown::Gone,
        };
        layout::head("Your account");
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::account::account_shown_page::render(&shown)
                .render()
                .into_inner()
        });
        self.framed(&account, "/account/devices", body).await
    }
}
