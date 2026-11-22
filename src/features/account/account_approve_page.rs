//! A new device asking to be let in, seen from a trusted one. What the
//! header's panel holds, as a page: where it lands with scripting off,
//! and where a refusal sends somebody back with its reason.

use hypertext::prelude::*;

use crate::features::account::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::infra::accounts::requests::DeviceRequest;
use crate::shell::components::account_control::{phone, ICON_LAPTOP, ICON_PHONE};
use crate::shell::components::passkey::Passkey as Prompt;
use crate::shell::layouts::layout;
use crate::shell::texts::sentences;

pub struct ApprovePage<'a> {
    pub request: Option<&'a DeviceRequest>,
    /// What the device signed in with, by the name the account knows.
    pub passkey: Option<&'a str>,
    pub trusted: bool,
    pub verify: Option<&'a Begun>,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ApprovePage<'a>) -> impl Renderable + 'a {
    let approve = page.request.map(|request| {
        let prompt = Prompt {
            verify: page.verify,
            ..Default::default()
        };
        let form = rsx! {
            <form method="post" action=(format!("/account/requests/{}/approve", request.id))>
                (prompt.fields())
                <button type="submit">(t("It was me"))</button>
            </form>
        }
        .render()
        .into_inner();
        let rendered = prompt.around(form).render().into_inner();
        rendered
    });
    rsx! {
    (layout::style_tag())
    <h1>(t("A device asking"))</h1>
    <div class="reading stack">
        @if let Some(message) = &page.error {
            (layout::error_note(message))
        }
        @if let Some(request) = page.request {
            <div class="items">
                <div class="item">
                    <span class="tile">
                        @if phone(&request.name) {
                            (hypertext::Raw::dangerously_create(ICON_PHONE))
                        } @else {
                            (hypertext::Raw::dangerously_create(ICON_LAPTOP))
                        }
                    </span>
                    <div>
                        <p class="item-name">(sentences::device(&request.name))</p>
                        <p class="item-meta">
                            (layout::when(request.created_at))
                            @if let Some(address) = &request.address {
                                " · "<span class="mono">(address)</span>
                            }
                            @if let Some(passkey) = page.passkey {
                                " · "(sentences::with_your_passkey(passkey))
                            }
                        </p>
                    </div>
                </div>
            </div>
            @if !page.trusted {
                <p class="field-hint">(t("Only a device you trust can let another in, and this one is not trusted."))</p>
            }
            <div class="actions">
                @if page.trusted {
                    @if let Some(form) = &approve {
                        (hypertext::Raw::dangerously_create(form))
                    }
                }
                <form method="post" action=(format!("/account/requests/{}/refuse", request.id))>
                    <button type="submit" class="btn btn-secondary">(t("It was not me"))</button>
                </form>
            </div>
        } @else {
            <p class="field-hint">(t("No device is asking."))</p>
        }
    </div>
    }
}
