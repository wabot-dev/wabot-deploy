//! Your account › Devices.

use hypertext::prelude::*;

use crate::features::account::account_tabs_component::tabs;
use crate::features::account::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::infra::accounts::devices::Device;
use crate::shell::components::passkey::Passkey as Prompt;
use crate::shell::layouts::layout;

pub struct DevicesPage<'a> {
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
        <form method="post" action="/account/devices/add" class="stack">
            (prompt.fields())
            <div class="actions">
                <button type="submit">(t("Add a device"))</button>
            </div>
        </form>
    }
    .render()
    .into_inner();
    let add = prompt.around(add).render().into_inner();
    rsx! {
    (layout::style_tag())
    (tabs("devices"))
    @if let Some(message) = &page.error {
        (layout::error_note(message))
    }
    <section class="stack">
        @if page.devices.is_empty() {
            <p class="field-hint">(t("No device is trusted. Signing in will need a link."))</p>
        } @else {
            <table>
                <thead>
                    <tr>
                        <th>(t("Device"))</th>
                        <th>(t("Last used"))</th>
                        <th>(t("From"))</th>
                        <th></th>
                    </tr>
                </thead>
                <tbody>
                    @for device in page.devices {
                        <tr>
                            <td>
                                (&device.name)
                                @if page.this_device == Some(device.id.as_str()) {
                                    " · "<span class="muted">(t("this device"))</span>
                                }
                            </td>
                            <td>(layout::when(device.last_used_at))</td>
                            <td>(device.last_address.as_deref().unwrap_or("—"))</td>
                            <td>
                                <form method="post" action=(format!("/account/devices/{}/delete", device.id))>
                                    <button type="submit" class="btn btn-secondary btn-sm">(t("Withdraw"))</button>
                                </form>
                            </td>
                        </tr>
                    }
                </tbody>
            </table>
        }
        @if page.trusted {
            (hypertext::Raw::dangerously_create(&add))
        }
    </section>
    }
}
