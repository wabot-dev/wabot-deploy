/// Which theme to paint in.
///
/// Stored on the account, so the attribute is in the first byte of
/// HTML the browser sees. A theme applied by script runs after the page
/// paints, and the flash of the wrong one is worse than not offering
/// the choice at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the operating system. The default, and the only answer
    /// that keeps up when somebody's machine switches at sunset.
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn parse(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}
