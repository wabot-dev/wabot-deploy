//! The account's icon in the header, and what it has to say.
//!
//! A device asking to sign in as somebody is answered from a device they
//! trust, and they are usually looking at some other page when it asks.
//! So the icon carries a dot on every page and opens a panel over it,
//! rather than a notice that pushed the page down and appeared only when
//! one loaded. The `account-live` island swaps in what `/account/live`
//! renders when it changes; without scripting it is current as of the
//! page, and the panel is a `popover`, which needs none.

use hypertext::prelude::*;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::infra::accounts::ceremonies::{self, Begun};
use crate::infra::accounts::requests::DeviceRequest;
use crate::infra::accounts::{events, passkeys, requests};
use crate::shell::components::passkey::Passkey;
use crate::shell::layouts::layout;
use crate::shell::texts::language::t;
use crate::shell::texts::sentences;

/// What the icon has to say, read cheaply enough to ask every few seconds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Glance {
    pub request: Option<DeviceRequest>,
    pub unseen: usize,
}

impl Glance {
    pub async fn read(database: &SqliteDatabase, account_id: &str) -> Self {
        Self {
            request: requests::pending_for(database, account_id)
                .await
                .ok()
                .flatten(),
            unseen: events::unseen(database, account_id).await.unwrap_or(0),
        }
    }

    /// Equal when nothing the icon shows has changed.
    pub fn key(&self) -> String {
        format!(
            "{}|{}",
            self.request
                .as_ref()
                .map(|request| request.id.as_str())
                .unwrap_or(""),
            self.unseen
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct AccountControl {
    pub username: String,
    /// On a page of the account itself, which lights the icon.
    pub here: bool,
    pub glance: Glance,
    /// What the requesting device signed in with, by the name the account
    /// knows it by.
    pub passkey: Option<String>,
    pub verify: Option<Begun>,
}

impl AccountControl {
    pub async fn load(database: &SqliteDatabase, account: &Account, here: bool) -> Self {
        let glance = Glance::read(database, &account.id).await;
        let (passkey, verify) = match &glance.request {
            Some(request) => (
                passkeys::find(database, &request.passkey_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|passkey| passkey.title().to_string()),
                ceremonies::begin_verify_anywhere(database, &account.id)
                    .await
                    .ok(),
            ),
            None => (None, None),
        };
        Self {
            username: account.username.clone(),
            here,
            glance,
            passkey,
            verify,
        }
    }

    /// The host the island hydrates, with what it swaps inside.
    pub fn render(&self) -> impl Renderable + '_ {
        let inner = self.inner();
        let class = match self.here {
            true => "account-control here",
            false => "account-control",
        };
        let wrapped = rsx! {
            <div class=(class) data-account-control>(hypertext::Raw::dangerously_create(&inner))</div>
        }
        .render()
        .into_inner();
        let props = serde_json::json!({
            "key": self.glance.key(),
            "asking": self.glance.request.as_ref().map(|request| request.id.as_str()),
        });
        wabot::ui::hypertext::island(
            "account-live",
            &props,
            hypertext::Raw::dangerously_create(wrapped),
        )
    }

    /// What `/account/live` sends when the glance changes.
    pub fn inner(&self) -> String {
        match &self.glance.request {
            Some(request) => self.asking(request),
            None => self.quiet(),
        }
    }

    fn quiet(&self) -> String {
        let (href, title) = match self.glance.unseen {
            0 => (
                "/account",
                format!("{} · {}", t("Your account"), self.username),
            ),
            count => ("/account/activity", sentences::new_in_activity(count)),
        };
        let unseen = self.glance.unseen > 0;
        rsx! {
            <a class="btn btn-ghost btn-icon account-button" href=(href)
               title=(&title) aria-label=(&title)>
                (hypertext::Raw::dangerously_create(ICON_ACCOUNT))
                @if unseen {
                    <span class="account-dot"></span>
                }
            </a>
        }
        .render()
        .into_inner()
    }

    fn asking(&self, request: &DeviceRequest) -> String {
        let prompt = Passkey {
            verify: self.verify.as_ref(),
            ..Default::default()
        };
        let approve = rsx! {
            <form method="post" action=(format!("/account/requests/{}/approve", request.id))>
                (prompt.fields())
                <button type="submit" class="btn btn-sm">(t("It was me"))</button>
            </form>
        }
        .render()
        .into_inner();
        let approve = prompt.around(approve).render().into_inner();
        let title = t("A device asks to sign in as you");
        let unseen = self.glance.unseen;
        let icon = match phone(&request.name) {
            true => ICON_PHONE,
            false => ICON_LAPTOP,
        };
        rsx! {
            <button type="button" class="btn btn-ghost btn-icon account-button"
                    popovertarget="account-panel" title=(title) aria-label=(title)>
                (hypertext::Raw::dangerously_create(ICON_ACCOUNT))
                <span class="account-dot asking"></span>
            </button>
            <div id="account-panel" popover class="account-panel">
                <div class="account-asking">
                    <span class="glyph">(hypertext::Raw::dangerously_create(icon))</span>
                    <div>
                        <p class="account-panel-name">(sentences::device(&request.name))</p>
                        <p class="account-panel-meta">
                            (t("asks to sign in as you"))" · "(layout::when(request.created_at))
                        </p>
                        <p class="account-panel-meta">
                            @if let Some(address) = &request.address {
                                <span class="mono">(address)</span>
                            }
                            @if request.address.is_some() && self.passkey.is_some() {
                                " · "
                            }
                            @if let Some(passkey) = &self.passkey {
                                (sentences::with_your_passkey(passkey))
                            }
                        </p>
                    </div>
                </div>
                <div class="account-panel-actions">
                    <form method="post" action=(format!("/account/requests/{}/refuse", request.id))>
                        <button type="submit" class="btn btn-ghost btn-sm destructive">(t("It was not me"))</button>
                    </form>
                    (hypertext::Raw::dangerously_create(&approve))
                </div>
                <div class="account-panel-foot">
                    @if unseen > 0 {
                        <a href="/account/activity">(sentences::new_in_activity(unseen))</a>
                    } @else {
                        <span></span>
                    }
                    <a href="/account">(t("Your account"))</a>
                </div>
            </div>
        }
        .render()
        .into_inner()
    }
}

/// Whether a browser's name is a phone's or a tablet's, for its icon.
pub fn phone(name: &str) -> bool {
    ["iPhone", "iPad", "Android"]
        .iter()
        .any(|word| name.contains(word))
}

/// XSS SAFETY: constants, never a value from a request. Raw because
/// `rsx!` does not know SVG's elements.
pub const ICON_ACCOUNT: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="8" r="4"/><path d="M4 21v-1a6 6 0 0 1 6-6h4a6 6 0 0 1 6 6v1"/></svg>"#;

pub const ICON_PHONE: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="7" y="2.5" width="10" height="19" rx="2.5"/><path d="M11 18.5h2"/></svg>"#;

pub const ICON_LAPTOP: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="5" width="16" height="10.5" rx="1.5"/><path d="M2 19h20"/></svg>"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn asking() -> AccountControl {
        AccountControl {
            username: "jorge".into(),
            here: false,
            glance: Glance {
                request: Some(DeviceRequest {
                    id: "req-1".into(),
                    account_id: "a".into(),
                    passkey_id: "pk".into(),
                    name: "Safari on iPhone".into(),
                    address: Some("181.52.10.4".into()),
                    approved: false,
                    created_at: crate::shell::state::now_ms(),
                    expires_at: crate::shell::state::now_ms() + 300_000,
                }),
                unseen: 0,
            },
            passkey: Some("Apple Passwords".into()),
            verify: None,
        }
    }

    /// The notice used to push every page down, and only appeared when one
    /// loaded. The icon is the same size either way and opens a panel over
    /// the page.
    #[test]
    fn a_device_asking_puts_a_dot_on_the_icon_and_its_answers_in_a_panel() {
        let html = asking().render().render().into_inner();

        assert!(html.contains(r#"popovertarget="account-panel""#), "{html}");
        assert!(html.contains("account-dot asking"), "{html}");
        assert!(html.contains(r#"id="account-panel" popover"#), "{html}");
        assert!(html.contains("/account/requests/req-1/approve"), "{html}");
        assert!(html.contains("/account/requests/req-1/refuse"), "{html}");
        assert!(html.contains("181.52.10.4"), "{html}");
        assert!(html.contains("Apple Passwords"), "{html}");
    }

    #[test]
    fn with_nothing_to_say_the_icon_is_a_link_to_the_account() {
        let html = AccountControl {
            username: "jorge".into(),
            ..Default::default()
        }
        .render()
        .render()
        .into_inner();

        assert!(html.contains(r#"href="/account""#), "{html}");
        assert!(!html.contains("account-dot"), "{html}");
        assert!(!html.contains("popover"), "{html}");
    }

    #[test]
    fn activity_not_yet_read_is_a_quiet_dot_that_goes_to_it() {
        let html = AccountControl {
            username: "jorge".into(),
            glance: Glance {
                request: None,
                unseen: 2,
            },
            ..Default::default()
        }
        .render()
        .render()
        .into_inner();

        assert!(html.contains(r#"href="/account/activity""#), "{html}");
        assert!(
            html.contains(r#"<span class="account-dot"></span>"#),
            "{html}"
        );
    }

    #[test]
    fn the_glance_changes_only_when_what_it_shows_does() {
        let one = asking().glance;
        let mut two = one.clone();
        assert_eq!(one.key(), two.key());
        two.unseen = 1;
        assert_ne!(one.key(), two.key());
        assert_ne!(one.key(), Glance::default().key());
    }
}
