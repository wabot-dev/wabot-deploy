//! The join token: everything the other node needs, in one paste.
//!
//! ## Why it is a blob and not six flags
//!
//! `wabot-deploy join --endpoint … --key … --address … --secret …` is
//! four chances to transcribe something wrong, and three of the four
//! failures do not show up until the tunnel does not come up. One
//! opaque string either arrives intact or does not parse.
//!
//! ## What is secret in here
//!
//! One field. The rest — an address, a hostname, a public key — is
//! ordinary, and none of it is worth protecting on its own. The token
//! as a whole is treated as the secret it contains: shown once, never
//! stored in clear, and out of URLs. Bundling them does not make the
//! public parts secret; it makes the secret part harder to separate
//! from the context that says what it opens.
//!
//! ## The version prefix
//!
//! `wdj1.` costs five bytes and buys two things: a paste that is
//! obviously a join token rather than a password, and a way for a
//! future field to be added without an older node reading the new token
//! as a corrupt old one.

use base64::Engine;
use serde::{Deserialize, Serialize};

/// What this version of a token looks like on the way in.
const PREFIX: &str = "wdj1.";

/// Everything the joining node is handed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinToken {
    /// The node that will send errands, by the id it calls itself.
    pub authority: String,
    /// What it calls itself, for the joining node's own list.
    pub name: String,
    /// `host:port` the control plane answers on. Not the overlay's
    /// endpoint, which is a different port and a different phase — this
    /// is where the callback goes.
    pub endpoint: String,
    /// The authority's overlay public key, so phase 2 has nothing left
    /// to exchange. See `network::keys`.
    pub public_key: String,
    /// The authority's own address on the overlay. Its key and its
    /// endpoint are useless without it — a tunnel needs to know which
    /// addresses live on the other side of it.
    pub overlay_ip: String,
    /// The address allocated to whoever spends this.
    pub assigned_ip: String,
    /// The one part worth protecting: what an errand from the authority
    /// will carry, and what the callback authenticates with.
    pub secret: String,
    /// What the minting node will ask of whoever spends this.
    ///
    /// The terms travel *with* the token, deliberately, so the node
    /// spending it can read them before it commits. Asking for them
    /// afterwards would be a consent screen for a decision already
    /// made — which is worse than no screen, because it looks like one.
    ///
    /// Absent on a token minted before this existed, and read as both
    /// capabilities: that is what those tokens meant.
    #[serde(default)]
    pub requires: Option<Vec<String>>,
    /// What the minting node offers in return — what it will let the
    /// joining node ask of *it*.
    #[serde(default)]
    pub offers: Option<Vec<String>>,
}

impl JoinToken {
    /// What this token asks for, in capabilities this version knows.
    pub fn requires(&self) -> Vec<super::capability::Capability> {
        Self::read(&self.requires)
    }

    /// What it hands over.
    pub fn offers(&self) -> Vec<super::capability::Capability> {
        Self::read(&self.offers)
    }

    /// An absent list is both, because a token minted before the terms
    /// existed granted everything. An *empty* list is empty — somebody
    /// chose that, and reading their "nothing" as "everything" would be
    /// the one mistake this whole phase exists to prevent.
    fn read(field: &Option<Vec<String>>) -> Vec<super::capability::Capability> {
        match field {
            None => super::capability::Capability::ALL.to_vec(),
            Some(names) => super::capability::parse_list(&names.join(",")),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("that does not look like a join token — they begin with `{PREFIX}`")]
    NotAToken,
    #[error("that join token is damaged; ask for another")]
    Damaged,
}

impl JoinToken {
    pub fn encode(&self) -> String {
        // Unpadded url-safe, so the whole thing survives being pasted
        // into a shell, a URL or a chat window without quoting.
        let json = serde_json::to_vec(self).expect("a token is plain data");
        format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
        )
    }

    /// Read one, refusing anything that is not exactly a token.
    ///
    /// Whitespace goes first: this arrives by copy and paste, and a
    /// trailing newline out of a terminal is not a damaged token.
    pub fn decode(text: &str) -> Result<Self, TokenError> {
        let body = text
            .trim()
            .strip_prefix(PREFIX)
            .ok_or(TokenError::NotAToken)?;
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| TokenError::Damaged)?;
        let token: Self = serde_json::from_slice(&json).map_err(|_| TokenError::Damaged)?;

        // A field that decoded to nothing would fail later, somewhere
        // that cannot say which one it was.
        let missing = token.authority.is_empty()
            || token.endpoint.is_empty()
            || token.public_key.is_empty()
            || token.overlay_ip.is_empty()
            || token.assigned_ip.is_empty()
            || token.secret.is_empty();
        if missing {
            return Err(TokenError::Damaged);
        }
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token() -> JoinToken {
        JoinToken {
            authority: "nd-abc123".into(),
            name: "wabot-deploy-testing.dev.tobaw.shop".into(),
            endpoint: "wabot-deploy-testing.dev.tobaw.shop:443".into(),
            public_key: "0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into(),
            overlay_ip: "10.42.0.1".into(),
            assigned_ip: "10.42.0.2".into(),
            secret: "a-very-long-secret".into(),
            requires: Some(vec!["host".into()]),
            offers: Some(vec!["edge".into()]),
        }
    }

    /// A token from before the terms existed granted everything, so
    /// that is what an absent list means. An **empty** one means empty:
    /// somebody chose that, and reading their "nothing" as "everything"
    /// is the one mistake this whole phase exists to prevent.
    #[test]
    fn an_absent_list_is_everything_and_an_empty_one_is_nothing() {
        let old = JoinToken {
            requires: None,
            offers: None,
            ..token()
        };
        let all = crate::network::capability::Capability::ALL.len();
        assert_eq!(old.requires().len(), all);
        assert_eq!(old.offers().len(), all);

        let refused = JoinToken {
            requires: Some(Vec::new()),
            offers: Some(Vec::new()),
            ..token()
        };
        assert!(refused.requires().is_empty());
        assert!(refused.offers().is_empty());
    }

    /// A capability a newer node knows and this one does not is left
    /// out, not refused: the rest of the terms are still readable, and
    /// a join that failed because one word was new would make every
    /// upgrade a flag day.
    #[test]
    fn a_capability_this_version_does_not_know_is_dropped() {
        let ahead = JoinToken {
            requires: Some(vec!["host".into(), "telepathy".into()]),
            ..token()
        };

        assert_eq!(
            ahead.requires(),
            vec![crate::network::capability::Capability::Host]
        );
    }

    #[test]
    fn a_token_survives_the_round_trip() {
        assert_eq!(
            JoinToken::decode(&token().encode()).expect("decode"),
            token()
        );
    }

    /// It arrives by copy and paste, so a newline out of a terminal is
    /// not a damaged token.
    #[test]
    fn whitespace_around_it_is_not_damage() {
        let encoded = token().encode();
        assert!(JoinToken::decode(&format!("  {encoded}\n")).is_ok());
    }

    /// One paste, quoted by nobody: a token with `+`, `/` or `=` in it
    /// is one somebody's shell will mangle.
    #[test]
    fn a_token_is_safe_to_paste_anywhere() {
        let encoded = token().encode();
        assert!(encoded.starts_with(PREFIX), "{encoded}");
        assert!(
            encoded[PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{encoded}"
        );
    }

    /// The two refusals say different things because they mean
    /// different things: one is the wrong string entirely, the other is
    /// the right kind of string that did not survive.
    #[test]
    fn what_is_not_a_token_says_which_way_it_is_wrong() {
        assert!(matches!(
            JoinToken::decode("hunter2"),
            Err(TokenError::NotAToken)
        ));
        assert!(matches!(
            JoinToken::decode("wdj1.not-base64-$$$"),
            Err(TokenError::Damaged)
        ));
        assert!(matches!(
            JoinToken::decode(&format!(
                "{PREFIX}{}",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("{}")
            )),
            Err(TokenError::Damaged)
        ));
    }

    /// A token missing a field would fail later, in the tunnel or the
    /// callback, where nothing can say which field it was.
    #[test]
    fn a_token_with_a_hole_in_it_is_damaged() {
        let mut token = token();
        token.secret = String::new();
        assert!(matches!(
            JoinToken::decode(&token.encode()),
            Err(TokenError::Damaged)
        ));
    }
}
