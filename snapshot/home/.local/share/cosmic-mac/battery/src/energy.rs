//! "Using Significant Energy": Linux has no per-app energy API, so approximate it
//! the way macOS mostly does in practice: processes holding a lot of CPU between
//! two samples of /proc.

use std::collections::HashMap;

/// Share of one CPU core, averaged since the previous sample, that counts as "significant".
const THRESHOLD_PERCENT: f64 = 25.0;

#[derive(Default)]
pub struct Sampler {
    prev_total: u64,
    prev: HashMap<u32, u64>,
}

impl Sampler {
    /// Returns process names above the threshold, heaviest first. The first call primes
    /// the sampler and returns nothing.
    pub fn sample(&mut self) -> Vec<String> {
        let Some(total) = total_jiffies() else { return Vec::new() };
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
        let elapsed = total.saturating_sub(self.prev_total) as f64;

        let mut now = HashMap::new();
        let mut heavy: HashMap<String, f64> = HashMap::new();
        for (pid, ticks) in process_jiffies() {
            if elapsed > 0.0 {
                if let Some(before) = self.prev.get(&pid) {
                    let pct = ticks.saturating_sub(*before) as f64 / elapsed * cores * 100.0;
                    if let Some(name) = comm(pid) {
                        *heavy.entry(name).or_default() += pct;
                    }
                }
            }
            now.insert(pid, ticks);
        }
        self.prev = now;
        self.prev_total = total;

        let mut heavy: Vec<_> = heavy.into_iter().filter(|(_, p)| *p >= THRESHOLD_PERCENT).collect();
        heavy.sort_by(|a, b| b.1.total_cmp(&a.1));
        heavy.into_iter().take(5).map(|(n, _)| n).collect()
    }
}

fn total_jiffies() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let cpu = stat.lines().next()?.strip_prefix("cpu ")?;
    Some(cpu.split_whitespace().filter_map(|v| v.parse::<u64>().ok()).sum())
}

fn process_jiffies() -> impl Iterator<Item = (u32, u64)> {
    std::fs::read_dir("/proc").into_iter().flatten().flatten().filter_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
        // fields after the parenthesised comm; utime/stime are fields 14/15 overall
        let rest = &stat[stat.rfind(')')? + 2..];
        let mut f = rest.split_whitespace().skip(11);
        let utime: u64 = f.next()?.parse().ok()?;
        let stime: u64 = f.next()?.parse().ok()?;
        Some((pid, utime + stime))
    })
}

fn comm(pid: u32) -> Option<String> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(name.trim().to_string())
}
