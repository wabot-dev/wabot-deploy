use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvGroup {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    pub variable: String,
    pub first: String,
    pub second: String,
}

impl Clash {
    pub fn sentence(&self) -> String {
        format!(
            "{} is set by both {} and {}, so which one a container got would be a guess — \
             a service joins only one group that sets a variable",
            self.variable, self.first, self.second
        )
    }
}

/// Refused rather than ordered: a precedence between groups is a rule
/// nobody reads until a container runs with the other value.
pub fn clash<'a>(groups: impl IntoIterator<Item = &'a EnvGroup>) -> Option<Clash> {
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for group in groups {
        for variable in group.env.keys() {
            if let Some(first) = seen.insert(variable, &group.name) {
                return Some(Clash {
                    variable: variable.clone(),
                    first: first.to_string(),
                    second: group.name.clone(),
                });
            }
        }
    }
    None
}

pub fn environment(
    groups: &[EnvGroup],
    own: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> =
        groups.iter().flat_map(|group| group.env.clone()).collect();
    env.extend(own.clone());
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, pairs: &[(&str, &str)]) -> EnvGroup {
        EnvGroup {
            id: format!("env-{name}"),
            project_id: "prj-1".into(),
            name: name.into(),
            env: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_services_own_variable_wins_over_its_groups() {
        let groups = [
            group("smtp", &[("SMTP_HOST", "mail"), ("SMTP_USER", "shared")]),
            group("sentry", &[("SENTRY_DSN", "https://key@sentry")]),
        ];
        let own: BTreeMap<String, String> = [("SMTP_USER".to_string(), "orders".to_string())]
            .into_iter()
            .collect();

        let env = environment(&groups, &own);

        assert_eq!(env.get("SMTP_HOST").map(String::as_str), Some("mail"));
        assert_eq!(env.get("SMTP_USER").map(String::as_str), Some("orders"));
        assert_eq!(
            env.get("SENTRY_DSN").map(String::as_str),
            Some("https://key@sentry")
        );
    }

    #[test]
    fn two_groups_setting_one_variable_are_named_with_it() {
        let groups = [
            group("smtp", &[("SMTP_HOST", "mail")]),
            group("smtp-eu", &[("SMTP_HOST", "mail.eu")]),
        ];
        let found = clash(&groups).expect("a clash");
        assert_eq!(found.variable, "SMTP_HOST");
        assert!(found.sentence().contains("smtp") && found.sentence().contains("smtp-eu"));

        assert_eq!(clash(&groups[..1]), None);
    }
}
