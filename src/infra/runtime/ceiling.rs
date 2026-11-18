use std::path::PathBuf;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// `memory.current` where the kernel is older than 5.19, which has no
/// peak: a reading now is a worse answer than the most ever used, and a
/// better one than none.
pub fn peak(pid: u32) -> Option<u64> {
    let directory = directory_of(pid)?;
    ["memory.peak", "memory.current"]
        .into_iter()
        .find_map(|file| parse(&std::fs::read_to_string(directory.join(file)).ok()?))
}

/// Written into the cgroup the process is in, rather than asking the
/// runtime: that is one path on systemd and on cgroupfs alike, and the
/// kernel's file is the thing a ceiling has to end up in either way.
pub fn bound(pid: u32, memory: u64) -> Result<(), String> {
    if cfg!(test) {
        return Err("a test does not change a container's cgroup".into());
    }
    let directory =
        directory_of(pid).ok_or_else(|| format!("process {pid} is in no cgroup to bound"))?;
    let write = |file: &str, value: &str| {
        let path = directory.join(file);
        std::fs::write(&path, value)
            .map_err(|error| format!("could not write {value:?} to {}: {error}", path.display()))
    };
    if directory.join("memory.swap.max").exists() {
        write("memory.swap.max", "0")?;
    }
    write("memory.max", &memory.to_string())
}

fn directory_of(pid: u32) -> Option<PathBuf> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    Some(PathBuf::from(CGROUP_ROOT).join(path.trim().trim_start_matches('/')))
}

fn parse(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_cannot_bound_a_real_process() {
        let refused = bound(std::process::id(), 256 * 1024 * 1024).expect_err("refused");
        assert!(refused.contains("test"), "{refused}");
    }

    #[test]
    fn a_reading_is_a_number_of_bytes_or_nothing() {
        assert_eq!(parse("272629760\n"), Some(272_629_760));
        assert_eq!(parse("max\n"), None);
        assert_eq!(parse(""), None);
    }
}
