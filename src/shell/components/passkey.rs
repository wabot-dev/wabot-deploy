//! A form that makes, gives or asks again for a passkey. The page renders
//! the challenge; the `passkey` island answers it and submits the form.
//! With scripting off the form still submits, without an answer.

use hypertext::prelude::*;

use crate::infra::accounts::ceremonies::Begun;
use crate::shell::texts::language::t;

/// The passkeys an account holds for this host, which the browser is told
/// after one is removed: a password manager that offers a key this
/// console no longer knows is one somebody picks and is refused.
#[derive(Clone, Copy)]
pub struct Known<'a> {
    pub host: &'a str,
    /// The account's id, which is every one of its passkeys' user handle.
    pub handle: &'a str,
    pub ids: &'a [&'a str],
}

#[derive(Default, Clone, Copy)]
pub struct Passkey<'a> {
    pub create: Option<&'a Begun>,
    pub get: Option<&'a Begun>,
    pub verify: Option<&'a Begun>,
    /// The field whose value names the account a passkey is made for.
    pub name_field: Option<&'a str>,
    /// The account's name, when there is no field for it.
    pub user: Option<&'a str>,
    /// Whether the form may go without a passkey when the browser cannot
    /// make one: setup, an invitation and a link can.
    pub optional: bool,
    /// Offer this host's passkeys in the username field's autofill as well
    /// as on the button. Signing in only.
    pub autofill: bool,
    pub known: Option<Known<'a>>,
}

impl Passkey<'_> {
    /// The fields the island fills. Inside the form.
    pub fn fields(&self) -> impl Renderable + '_ {
        let answer = self.create.or(self.get);
        rsx! {
            @if let Some(begun) = answer {
                <input type="hidden" name="challenge" value=(&begun.challenge)>
                <input type="hidden" name="credential" value="">
            }
            @if let Some(begun) = self.verify {
                <input type="hidden" name="verify_challenge" value=(&begun.challenge)>
                <input type="hidden" name="verify_credential" value="">
            }
        }
    }

    /// The island around a form whose fields came from [`Self::fields`].
    pub fn around(&self, form: String) -> impl Renderable + '_ {
        let known = self.known.map(|known| {
            serde_json::json!({
                "rpId": known.host,
                "userId": wabot_addon_auth_passkey::to_base64url(known.handle.as_bytes()),
                "ids": known.ids,
            })
        });
        let props = serde_json::json!({
            "create": self.create.map(|begun| &begun.options),
            "get": self.get.map(|begun| &begun.options),
            "verify": self.verify.map(|begun| &begun.options),
            "name": self.name_field,
            "user": self.user,
            "optional": self.optional,
            "autofill": self.autofill,
            "known": known,
            "failed": t("Your device gave no passkey. Try again."),
            "exists": t("This device already has a passkey for your account."),
            "unsupported": t("This browser cannot use passkeys."),
        });
        let inner = rsx! {
            (hypertext::Raw::dangerously_create(&form))
            <p class="field-error" data-passkey-error hidden></p>
        }
        .render()
        .into_inner();
        wabot::ui::hypertext::island("passkey", &props, hypertext::Raw::dangerously_create(inner))
    }
}
