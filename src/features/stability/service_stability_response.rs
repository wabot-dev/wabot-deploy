use crate::domain::entities::copy_metric::CopyMetric;
use crate::domain::entities::service_event::{EventKind, Severity};
use crate::domain::policies::stability::{Day, Row};
use crate::domain::values::time_zone::TimeZone;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chart {
    Memory,
    Cpu,
    Network,
    Disk,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyKey {
    pub slot: u32,
    pub node: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLine {
    pub id: String,
    pub kind: EventKind,
    pub severity: Severity,
    pub at: i64,
    pub until: Option<i64>,
    pub slot: Option<u32>,
    pub node: Option<String>,
    pub code: Option<i64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceStabilityResponse {
    pub days: u32,
    pub zone: TimeZone,
    pub today: i64,
    pub selected: i64,
    /// The first ten minutes the charts draw: the selected day's midnight,
    /// back to the ten minutes holding it, because a zone can be a quarter
    /// of an hour off them.
    pub start: i64,
    /// How many ten minutes the selected day spans from `start`, which is
    /// not 144 on a day the clocks change.
    pub span: usize,
    pub since: Option<i64>,
    pub rows: Vec<Row>,
    pub strip: Vec<Day>,
    pub deploys: Vec<(i64, String)>,
    pub copies: Vec<CopyKey>,
    pub buckets: usize,
    pub series: Vec<Vec<Option<CopyMetric>>>,
    pub charts: Vec<Chart>,
    pub day_events: Vec<EventLine>,
    pub period_events: Vec<EventLine>,
    pub pinned: Option<String>,
    pub unreachable: Vec<(String, String)>,
}
