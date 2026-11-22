//! Which public names an organisation's project may answer on.
//!
//! A browser decides `SameSite` by registrable domain, so a page served
//! under the node's domain is the same site as the node's console, and a
//! page under another organisation's domain is the same site as that
//! organisation's. See `docs/access.md`, "Same site".

pub const UNDER_THE_NODE: &str =
    "that name is under this node's own domain — an organisation's services answer under its own \
     domain, or one of their own";
pub const UNDER_ANOTHER: &str = "that name is under another organisation's domain";

/// Why an organisation's project may not answer on `hostname`, if it may
/// not.
///
/// Its own domain always wins, because an organisation's domain may itself
/// be a subdomain of the node's.
pub fn refusal(
    hostname: &str,
    node_domain: Option<&str>,
    own_domain: Option<&str>,
    others: &[String],
) -> Option<&'static str> {
    let name = hostname
        .trim()
        .trim_start_matches("*.")
        .to_ascii_lowercase();
    if own_domain.is_some_and(|own| under(&name, own)) {
        return None;
    }
    if others.iter().any(|other| under(&name, other)) {
        return Some(UNDER_ANOTHER);
    }
    if node_domain.is_some_and(|node| under(&name, node)) {
        return Some(UNDER_THE_NODE);
    }
    None
}

fn under(name: &str, domain: &str) -> bool {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    !domain.is_empty() && (name == domain || name.ends_with(&format!(".{domain}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: Option<&str> = Some("node.example.com");

    #[test]
    fn a_name_under_the_node_is_refused() {
        assert_eq!(
            refusal("shop.acme.node.example.com", NODE, None, &[]),
            Some(UNDER_THE_NODE)
        );
        assert_eq!(
            refusal("node.example.com", NODE, None, &[]),
            Some(UNDER_THE_NODE)
        );
        assert_eq!(
            refusal("*.apps.node.example.com", NODE, None, &[]),
            Some(UNDER_THE_NODE)
        );
    }

    #[test]
    fn a_name_of_their_own_is_theirs() {
        assert_eq!(
            refusal("shop.example.org", NODE, Some("acme.com"), &[]),
            None
        );
        assert_eq!(refusal("www.acme.com", NODE, Some("acme.com"), &[]), None);
    }

    #[test]
    fn their_own_domain_wins_when_it_sits_under_the_node() {
        let own = Some("acme.node.example.com");
        assert_eq!(refusal("shop.acme.node.example.com", NODE, own, &[]), None);
        assert_eq!(
            refusal("shop.globex.node.example.com", NODE, own, &[]),
            Some(UNDER_THE_NODE)
        );
    }

    #[test]
    fn a_name_under_another_organisation_is_refused() {
        let others = ["globex.com".to_string()];
        assert_eq!(
            refusal("login.globex.com", NODE, Some("acme.com"), &others),
            Some(UNDER_ANOTHER)
        );
        assert_eq!(
            refusal("notglobex.com", NODE, Some("acme.com"), &others),
            None
        );
    }
}
