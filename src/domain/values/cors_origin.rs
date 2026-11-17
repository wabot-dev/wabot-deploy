use std::fmt;

use serde::Serialize;

use crate::domain::values::zone_name::is_host_label;

pub const NOT_AN_ORIGIN: &str =
    "an origin is a scheme and a host, like https://app.example.com, or * for any";
pub const PATH: &str =
    "an origin has no path: https://app.example.com, not https://app.example.com/upload";

/// Where a browser page that may call a bucket is served from.
///
/// What a browser sends in `Origin`, so it is compared as that: a scheme,
/// a host and a port, never a path. `*` is any page anywhere, and
/// `https://*.example.com` any page under one domain — the one wildcard S3
/// reads in an origin.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct CorsOrigin(String);

impl CorsOrigin {
    pub fn parse(typed: &str) -> Result<Self, &'static str> {
        let typed = typed.trim();
        if typed == "*" {
            return Ok(Self(typed.to_string()));
        }
        let lowered = typed.to_ascii_lowercase();
        let (scheme, rest) = lowered.split_once("://").ok_or(NOT_AN_ORIGIN)?;
        if scheme != "https" && scheme != "http" {
            return Err(NOT_AN_ORIGIN);
        }
        let authority = rest.strip_suffix('/').unwrap_or(rest);
        if authority.contains('/') || authority.contains('?') || authority.contains('#') {
            return Err(PATH);
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if port.is_some_and(|port| port.parse::<u16>().map_or(true, |port| port == 0)) {
            return Err(NOT_AN_ORIGIN);
        }
        let named = host.strip_prefix("*.").unwrap_or(host);
        if named.is_empty() || !named.split('.').all(is_host_label) {
            return Err(NOT_AN_ORIGIN);
        }
        Ok(Self(format!("{scheme}://{authority}")))
    }

    /// One per line, blank lines skipped: what a textarea sends.
    pub fn parse_lines(typed: &str) -> Result<Vec<Self>, &'static str> {
        let mut origins: Vec<Self> = typed
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(Self::parse)
            .collect::<Result<_, _>>()?;
        origins.sort();
        origins.dedup();
        Ok(origins)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CorsOrigin {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(typed: &str) -> String {
        CorsOrigin::parse(typed)
            .expect("an origin")
            .as_str()
            .to_string()
    }

    #[test]
    fn an_origin_is_compared_as_a_browser_sends_it() {
        assert_eq!(origin("https://app.example.com"), "https://app.example.com");
        assert_eq!(
            origin(" HTTPS://App.Example.com/ "),
            "https://app.example.com"
        );
        assert_eq!(origin("http://localhost:5173"), "http://localhost:5173");
    }

    #[test]
    fn any_page_and_any_page_under_a_domain_are_the_two_wildcards() {
        assert_eq!(origin("*"), "*");
        assert_eq!(origin("https://*.example.com"), "https://*.example.com");
        assert_eq!(CorsOrigin::parse("https://app.*.com"), Err(NOT_AN_ORIGIN));
    }

    #[test]
    fn a_url_with_a_path_is_refused_with_what_to_type_instead() {
        assert_eq!(
            CorsOrigin::parse("https://app.example.com/upload"),
            Err(PATH)
        );
        assert_eq!(CorsOrigin::parse("https://app.example.com?x=1"), Err(PATH));
    }

    #[test]
    fn what_is_not_an_origin_says_what_one_is() {
        for typed in [
            "app.example.com",
            "ftp://app.example.com",
            "https://",
            "https://app.example.com:0",
            "https://app.example.com:99999",
            "https://-app.example.com",
        ] {
            assert_eq!(CorsOrigin::parse(typed), Err(NOT_AN_ORIGIN), "{typed}");
        }
    }

    #[test]
    fn a_textarea_becomes_each_origin_once() {
        let origins = CorsOrigin::parse_lines(
            "https://b.example.com\n\n  https://a.example.com\nhttps://B.example.com/\n",
        )
        .expect("origins");
        let spelled: Vec<&str> = origins.iter().map(CorsOrigin::as_str).collect();
        assert_eq!(spelled, ["https://a.example.com", "https://b.example.com"]);
        assert_eq!(CorsOrigin::parse_lines(""), Ok(Vec::new()));
        assert_eq!(
            CorsOrigin::parse_lines("https://a.example.com\nnope"),
            Err(NOT_AN_ORIGIN)
        );
    }
}
