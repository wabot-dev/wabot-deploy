use serde::{Deserialize, Serialize};

pub const BUCKET_MS: i64 = 600_000;

pub fn bucket_of(at: i64) -> i64 {
    at - at.rem_euclid(BUCKET_MS)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyMetric {
    pub container: String,
    pub bucket: i64,
    pub samples: u32,
    pub running: u32,
    pub memory_max: Option<u64>,
    pub memory_limit: Option<u64>,
    pub cpu_avg: Option<u32>,
    pub cpu_max: Option<u32>,
    pub cpu_limit: Option<u32>,
    pub throttled: Option<u32>,
    pub received: Option<u64>,
    pub sent: Option<u64>,
    pub read: Option<u64>,
    pub written: Option<u64>,
    pub disk: Option<u64>,
    pub disk_partial: bool,
    pub disk_limit: Option<u64>,
}

impl CopyMetric {
    pub fn down_minutes(&self) -> u32 {
        self.samples.saturating_sub(self.running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moment_falls_in_the_ten_minutes_that_began_before_it() {
        assert_eq!(bucket_of(1_200_000), 1_200_000);
        assert_eq!(bucket_of(1_799_999), 1_200_000);
        assert_eq!(bucket_of(1_800_000), 1_800_000);
    }
}
