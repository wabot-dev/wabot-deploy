//! One copy's counters, read from its cgroup and its network namespace.
//! Read only: nothing here writes to the machine.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub pid: u32,
    pub memory: u64,
    pub memory_limit: Option<u64>,
    pub cpu_limit: Option<u32>,
    pub cpu_usec: u64,
    pub throttled_periods: u64,
    pub period_usec: u64,
    pub received: Option<u64>,
    pub sent: Option<u64>,
    pub read: Option<u64>,
    pub written: Option<u64>,
}

pub fn read(pid: u32, cgroup: &str) -> Option<Reading> {
    let directory = format!("/sys/fs/cgroup{cgroup}");
    let file = |name: &str| std::fs::read_to_string(format!("{directory}/{name}")).ok();
    let memory = file("memory.current")?.trim().parse().ok()?;
    let cpu = file("cpu.stat")
        .map(|text| cpu_stat(&text))
        .unwrap_or_default();
    let (cpu_limit, period_usec) = file("cpu.max")
        .map(|text| cpu_max(&text))
        .unwrap_or((None, 100_000));
    let io = file("io.stat").map(|text| io_stat(&text));
    let net = std::fs::read_to_string(format!("/proc/{pid}/net/dev"))
        .ok()
        .map(|text| net_dev(&text));
    Some(Reading {
        pid,
        memory,
        memory_limit: file("memory.max").and_then(|text| text.trim().parse().ok()),
        cpu_limit,
        cpu_usec: cpu.0,
        throttled_periods: cpu.1,
        period_usec,
        received: net.map(|(received, _)| received),
        sent: net.map(|(_, sent)| sent),
        read: io.map(|(read, _)| read),
        written: io.map(|(_, written)| written),
    })
}

fn cpu_stat(text: &str) -> (u64, u64) {
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.trim().parse().ok())
            .unwrap_or(0)
    };
    (field("usage_usec "), field("nr_throttled "))
}

fn cpu_max(text: &str) -> (Option<u32>, u64) {
    let mut words = text.split_whitespace();
    let quota = words.next().and_then(|quota| quota.parse::<u64>().ok());
    let period = words
        .next()
        .and_then(|period| period.parse::<u64>().ok())
        .filter(|period| *period > 0)
        .unwrap_or(100_000);
    (
        quota.and_then(|quota| u32::try_from(quota * 1_000 / period).ok()),
        period,
    )
}

fn io_stat(text: &str) -> (u64, u64) {
    let mut totals = (0, 0);
    for line in text.lines() {
        for word in line.split_whitespace().skip(1) {
            if let Some(bytes) = word
                .strip_prefix("rbytes=")
                .and_then(|v| v.parse::<u64>().ok())
            {
                totals.0 += bytes;
            }
            if let Some(bytes) = word
                .strip_prefix("wbytes=")
                .and_then(|v| v.parse::<u64>().ok())
            {
                totals.1 += bytes;
            }
        }
    }
    totals
}

/// Seen from inside the container, so received is what came into it.
fn net_dev(text: &str) -> (u64, u64) {
    let mut totals = (0, 0);
    for line in text.lines().skip(2) {
        let Some((name, counters)) = line.split_once(':') else {
            continue;
        };
        if name.trim() == "lo" {
            continue;
        }
        let numbers: Vec<u64> = counters
            .split_whitespace()
            .filter_map(|word| word.parse().ok())
            .collect();
        if numbers.len() >= 9 {
            totals.0 += numbers[0];
            totals.1 += numbers[8];
        }
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_stat_gives_the_time_used_and_the_periods_held_back() {
        let text = "usage_usec 8123456\nuser_usec 6000000\nsystem_usec 2123456\n\
                    nr_periods 900\nnr_throttled 42\nthrottled_usec 1500000\n";
        assert_eq!(cpu_stat(text), (8_123_456, 42));
    }

    #[test]
    fn a_ceiling_of_half_a_core_reads_as_five_hundred_millicores() {
        assert_eq!(cpu_max("50000 100000\n"), (Some(500), 100_000));
        assert_eq!(cpu_max("max 100000\n"), (None, 100_000));
    }

    #[test]
    fn io_stat_adds_every_device() {
        let text = "253:0 rbytes=1000 wbytes=2000 rios=1 wios=2 dbytes=0 dios=0\n\
                    8:0 rbytes=10 wbytes=20 rios=1 wios=1 dbytes=0 dios=0\n";
        assert_eq!(io_stat(text), (1010, 2020));
    }

    #[test]
    fn the_loopback_is_not_traffic() {
        let text = "Inter-|   Receive                                                |  Transmit\n \
                    face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    \
                    lo: 5000 10 0 0 0 0 0 0 5000 10 0 0 0 0 0 0\n  \
                    eth0: 1200 8 0 0 0 0 0 0 3400 9 0 0 0 0 0 0\n";
        assert_eq!(net_dev(text), (1200, 3400));
    }
}
