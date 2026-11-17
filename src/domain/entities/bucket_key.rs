//! A key that opens one bucket.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    ReadWrite,
    ReadOnly,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::ReadWrite => "read_write",
            Access::ReadOnly => "read_only",
        }
    }

    /// A value this node cannot read is read-only: granting writes nobody
    /// asked for is the wrong way to be wrong.
    pub fn parse(text: &str) -> Self {
        match text {
            "read_write" => Access::ReadWrite,
            _ => Access::ReadOnly,
        }
    }

    pub fn can_write(self) -> bool {
        self == Access::ReadWrite
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketKey {
    pub id: String,
    pub bucket_id: String,
    pub name: String,
    pub access_key_id: String,
    pub secret_key: String,
    pub access: Access,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_access_reads_back_as_it_was_written() {
        for access in [Access::ReadWrite, Access::ReadOnly] {
            assert_eq!(Access::parse(access.as_str()), access);
        }
    }

    #[test]
    fn an_access_this_node_does_not_know_cannot_write() {
        assert_eq!(Access::parse("owner"), Access::ReadOnly);
        assert_eq!(Access::parse(""), Access::ReadOnly);
        assert!(!Access::parse("anything").can_write());
    }
}
