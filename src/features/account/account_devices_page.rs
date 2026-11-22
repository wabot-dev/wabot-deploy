//! Your account › Devices.

use hypertext::prelude::*;

use crate::features::account::account_tabs_component::tabs;
use crate::features::account::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::infra::accounts::devices::Device;
use crate::shell::components::account_control::{phone, ICON_LAPTOP, ICON_PHONE};
use crate::shell::components::passkey::Passkey as Prompt;
use crate::shell::layouts::layout;
use crate::shell::texts::sentences;

pub struct DevicesPage<'a> {
    pub username: &'a str,
    pub devices: &'a [Device],
    pub this_device: Option<&'a str>,
    /// Whether this session is on a trusted device, which is what may add
    /// another.
    pub trusted: bool,
    pub verify: Option<&'a Begun>,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a DevicesPage<'a>) -> impl Renderable + 'a {
    let prompt = Prompt {
        verify: page.verify,
        ..Default::default()
    };
    let add = rsx! {
        <form method="post" action="/account/devices/add">
            (prompt.fields())
            <button type="submit" class="btn btn-sm">(t("Add a device"))</button>
        </form>
    }
    .render()
    .into_inner();
    let add = prompt.around(add).render().into_inner();
    let count = sentences::trusted_devices(page.devices.len());
    rsx! {
    (layout::style_tag())
    (tabs("devices", page.username))
    <div class="reading stack">
        @if let Some(message) = &page.error {
            (layout::error_note(message))
        }
        <div class="list-head">
            <p class="card-label">(count)</p>
            @if page.trusted {
                (hypertext::Raw::dangerously_create(&add))
            }
        </div>
        @if page.devices.is_empty() {
            <p class="field-hint">(t("No device is trusted. Signing in will need a link."))</p>
        } @else {
            <div class="items">
                @for device in page.devices {
                    <div class="item">
                        <span class="glyph">
                            @if phone(&device.name) {
                                (hypertext::Raw::dangerously_create(ICON_PHONE))
                            } @else {
                                (hypertext::Raw::dangerously_create(ICON_LAPTOP))
                            }
                        </span>
                        <div>
                            <p class="item-name">
                                (sentences::device(&device.name))
                                @if page.this_device == Some(device.id.as_str()) {
                                    <span class="badge badge-brand">(t("this device"))</span>
                                }
                            </p>
                            <p class="item-meta">
                                (sentences::trusted_since(&layout::when(device.created_at)))
                                " · "(sentences::last_used(&layout::when(device.last_used_at)))
                                @if let Some(address) = &device.last_address {
                                    " · "<span class="mono">(address)</span>
                                }
                            </p>
                        </div>
                        <div class="item-actions">
                            <form method="post" action=(format!("/account/devices/{}/delete", device.id))>
                                <button type="submit" class="btn btn-ghost btn-sm">(t("Stop trusting"))</button>
                            </form>
                        </div>
                    </div>
                }
            </div>
        }
    </div>
    }
}
