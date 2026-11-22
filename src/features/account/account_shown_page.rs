//! Shown once: the code a new device types, or the link a phone opens.

use hypertext::prelude::*;

use crate::features::account::texts::t;
use crate::shell::components::countdown::countdown;
use crate::shell::layouts::layout;
use crate::shell::texts::sentences;

pub enum Shown<'a> {
    Code {
        code: &'a str,
        name: &'a str,
        left: i64,
    },
    Link {
        url: &'a str,
        qr: &'a str,
        left: i64,
    },
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

/// Six digits read more easily as two threes, and are typed as six.
fn grouped(code: &str) -> String {
    match code.len() {
        6 => format!("{} {}", &code[..3], &code[3..]),
        _ => code.to_string(),
    }
}

pub fn render<'a>(shown: &'a Shown<'a>) -> impl Renderable + 'a {
    rsx! {
    (layout::style_tag())
    @match shown {
        Shown::Code { code, name, left } => {
            <div class="reading stack">
                <h1>(sentences::type_it_in(&sentences::device(name)))</h1>
                <p class="code-big">(grouped(code))</p>
                <p class="item-meta">(countdown(*left)) " · " (t("works once"))</p>
            </div>
        }
        Shown::Link { url, qr, left } => {
            <div class="reading stack">
                <h1>(t("Open it on the new device"))</h1>
                <div class="qr-line">
                    <div class="qr">(hypertext::Raw::dangerously_create(qr))</div>
                    <div class="stack-sm">
                        (copyable(url))
                        <p class="item-meta">(countdown(*left)) " · " (t("works once"))</p>
                        <p class="item-meta">(t("It makes its own passkey there, and is trusted."))</p>
                    </div>
                </div>
            </div>
        }
        Shown::Gone => {
            <div class="reading stack">
                <p class="field-hint">(t("That was shown once, and is gone."))</p>
                <p><a href="/account">(t("Your account"))</a></p>
            </div>
        }
    }
    }
}
