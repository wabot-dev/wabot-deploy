//! Shown once: the code a new device types, or the link a phone opens.

use hypertext::prelude::*;

use crate::features::account::texts::t;
use crate::shell::layouts::layout;

pub enum Shown<'a> {
    Code { code: &'a str, name: &'a str },
    Link { url: &'a str, qr: &'a str },
    Gone,
}

fn copyable(value: &str) -> impl Renderable + '_ {
    let inner = rsx! {
        <div class="dsn-line">
            <pre class="dsn-value" data-copy
                 data-copy-label=(t("Copy")) data-copied-label=(t("Copied"))>(value)</pre>
        </div>
    }
    .render()
    .into_inner();
    wabot::ui::hypertext::island(
        "copy",
        &serde_json::json!({}),
        hypertext::Raw::dangerously_create(&inner),
    )
}

pub fn render<'a>(shown: &'a Shown<'a>) -> impl Renderable + 'a {
    rsx! {
    (layout::style_tag())
    @match shown {
        Shown::Code { code, name } => {
            <h1>(t("Type this on the other device"))</h1>
            <section class="card stack">
                <p>(name)</p>
                <pre class="code-big">(code)</pre>
            </section>
        }
        Shown::Link { url, qr } => {
            <h1>(t("Add a device"))</h1>
            <section class="card stack">
                <p>(t("Open this on the new device within five minutes. It makes its own passkey there, and is trusted."))</p>
                <div class="qr">(hypertext::Raw::dangerously_create(qr))</div>
                (copyable(url))
            </section>
        }
        Shown::Gone => {
            <p class="field-hint">(t("That was shown once, and is gone."))</p>
            <p><a href="/account">(t("Your account"))</a></p>
        }
    }
    }
}
