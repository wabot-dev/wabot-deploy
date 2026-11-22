//! A new device asking to be let in, seen from a trusted one.

use hypertext::prelude::*;

use crate::features::account::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::infra::accounts::requests::DeviceRequest;
use crate::shell::components::passkey::Passkey as Prompt;
use crate::shell::layouts::layout;

pub struct ApprovePage<'a> {
    pub request: Option<&'a DeviceRequest>,
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
    @if let Some(message) = &page.error {
        (layout::error_note(message))
    }
    @if let Some(request) = page.request {
        <section class="card stack">
            <p>
                <strong>(&request.name)</strong>
                @if let Some(address) = &request.address {
                    " · "(address)
                }
                " · "(layout::when(request.created_at))
            </p>
            <p class="field-hint">(t("Its passkey passed. If it was you, it shows six digits to type there."))</p>
            <div class="actions">
                @if page.trusted {
                    @if let Some(form) = &approve {
                        (hypertext::Raw::dangerously_create(form))
                    }
                } @else {
                    <p class="field-hint">(t("Only a device you trust can let another in, and this one is not trusted."))</p>
                }
                <form method="post" action=(format!("/account/requests/{}/refuse", request.id))>
                    <button type="submit" class="btn btn-secondary">(t("It was not me"))</button>
                </form>
            </div>
        </section>
    } @else {
        <p class="field-hint">(t("No device is asking."))</p>
    }
    }
}
