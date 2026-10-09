use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    En,
    Es,
}

impl Language {
    pub fn parse(value: &str) -> Self {
        match value {
            "es" => Self::Es,
            _ => Self::En,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Es => "es",
        }
    }
}
