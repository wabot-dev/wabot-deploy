use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Deployed,
    Stopped,
    Started,
    Crashed,
    OomKilled,
    BackedUp,
    BackupFailed,
    Unfollowed,
    Unmeasured,
    Down,
    Throttled,
    MemoryHigh,
    DiskHigh,
    EnvChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Serious,
    Critical,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Deployed => "deployed",
            EventKind::Stopped => "stopped",
            EventKind::Started => "started",
            EventKind::Crashed => "crashed",
            EventKind::OomKilled => "oom_killed",
            EventKind::BackedUp => "backed_up",
            EventKind::BackupFailed => "backup_failed",
            EventKind::Unfollowed => "unfollowed",
            EventKind::Unmeasured => "unmeasured",
            EventKind::Down => "down",
            EventKind::Throttled => "throttled",
            EventKind::MemoryHigh => "memory_high",
            EventKind::DiskHigh => "disk_high",
            EventKind::EnvChanged => "env_changed",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        ALL.iter().copied().find(|kind| kind.as_str() == text)
    }

    pub fn severity(self, code: Option<i64>) -> Severity {
        match self {
            EventKind::Deployed
            | EventKind::Stopped
            | EventKind::Started
            | EventKind::BackedUp
            | EventKind::EnvChanged => Severity::Info,
            EventKind::Throttled | EventKind::MemoryHigh => Severity::Warning,
            EventKind::DiskHigh if code.unwrap_or(0) < 95 => Severity::Warning,
            EventKind::Crashed | EventKind::BackupFailed => Severity::Serious,
            EventKind::DiskHigh
            | EventKind::OomKilled
            | EventKind::Unfollowed
            | EventKind::Unmeasured
            | EventKind::Down => Severity::Critical,
        }
    }

    pub fn routine(self) -> bool {
        self == EventKind::BackedUp
    }
}

pub const ALL: [EventKind; 14] = [
    EventKind::Deployed,
    EventKind::Stopped,
    EventKind::Started,
    EventKind::Crashed,
    EventKind::OomKilled,
    EventKind::BackedUp,
    EventKind::BackupFailed,
    EventKind::Unfollowed,
    EventKind::Unmeasured,
    EventKind::Down,
    EventKind::Throttled,
    EventKind::MemoryHigh,
    EventKind::DiskHigh,
    EventKind::EnvChanged,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEvent {
    pub id: String,
    pub service_id: String,
    pub container: Option<String>,
    pub kind: EventKind,
    pub at: i64,
    pub until: Option<i64>,
    pub code: Option<i64>,
    pub detail: Option<String>,
}

impl ServiceEvent {
    pub fn severity(&self) -> Severity {
        self.kind.severity(self.code)
    }

    pub fn ends(&self) -> i64 {
        self.until.unwrap_or(self.at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_read_back_as_the_name_it_is_stored_under() {
        for kind in ALL {
            assert_eq!(EventKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(EventKind::parse("nonsense"), None);
    }

    #[test]
    fn a_disk_past_ninety_five_is_worse_than_one_past_eighty_five() {
        assert_eq!(EventKind::DiskHigh.severity(Some(85)), Severity::Warning);
        assert_eq!(EventKind::DiskHigh.severity(Some(95)), Severity::Critical);
    }
}
