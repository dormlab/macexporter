//! CPU usage and memory used, both parsed from a single `top -l 1 -n 0`.
//!
//! `top`'s `PhysMem:` line on Apple Silicon looks like:
//!   `PhysMem: 15G used (4801M wired, 5292M compressor), 91M unused.`
//!
//! `top`'s CPU line looks like:
//!   `CPU usage: 5.55% user, 4.44% sys, 89.99% idle`
//!
//! We define `dormlab_memory_used_bytes` as `wired + compressor` rather
//! than `top`'s headline `15G used`. That headline includes file-backed
//! cache that's reclaimable; users care about non-reclaimable in-use
//! memory when looking at headroom.

use anyhow::Result;

use super::run_cmd;

pub async fn cpu_and_memory() -> Result<(Option<f64>, Option<u64>)> {
    let out = run_cmd("top", &["-l", "1", "-n", "0"]).await?;
    Ok((parse_cpu(&out), parse_memory_used(&out)))
}

fn parse_cpu(out: &str) -> Option<f64> {
    for line in out.lines() {
        let Some(rest) = line.strip_prefix("CPU usage: ") else {
            continue;
        };
        let mut user: Option<f64> = None;
        let mut sys: Option<f64> = None;
        for part in rest.split(',') {
            let part = part.trim();
            if let Some(num) = part.strip_suffix("% user") {
                user = num.trim().parse().ok();
            } else if let Some(num) = part.strip_suffix("% sys") {
                sys = num.trim().parse().ok();
            }
        }
        return Some((user? + sys?) / 100.0);
    }
    None
}

fn parse_memory_used(out: &str) -> Option<u64> {
    for line in out.lines() {
        let Some(rest) = line.strip_prefix("PhysMem:") else {
            continue;
        };
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        let inner = &rest[open + 1..close];
        let mut wired: Option<u64> = None;
        let mut compressor: Option<u64> = None;
        for part in inner.split(',') {
            let part = part.trim();
            if let Some(n) = part.strip_suffix(" wired") {
                wired = parse_size(n);
            } else if let Some(n) = part.strip_suffix(" compressor") {
                compressor = parse_size(n);
            }
        }
        return Some(wired? + compressor?);
    }
    None
}

/// Parse strings like `4801M`, `5.2G`, `123K`, returning bytes.
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, unit) = s.split_at(s.len() - 1);
    let n: f64 = num.parse().ok()?;
    let mult: u64 = match unit {
        "K" => 1 << 10,
        "M" => 1 << 20,
        "G" => 1 << 30,
        "T" => 1 << 40,
        _ => return None,
    };
    Some((n * mult as f64) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Processes: 588 total, 3 running, 585 sleeping, 4220 threads
2026/04/27 17:00:00
Load Avg: 1.20, 1.30, 1.50
CPU usage: 4.55% user, 5.50% sys, 89.95% idle
SharedLibs: 240M resident, 47M data, 5M linkedit.
MemRegions: 55144 total, 779M resident, 56M private, 2994M shared.
PhysMem: 15G used (4801M wired, 5292M compressor), 91M unused.
VM: 256T vsize, 4395M framework vsize, 0(0) swapins, 0(0) swapouts.
";

    #[test]
    fn parses_cpu() {
        let cpu = parse_cpu(SAMPLE).unwrap();
        // 4.55 + 5.5 = 10.05 of 100 = 0.1005
        assert!((cpu - 0.1005).abs() < 1e-6, "got {cpu}");
    }

    #[test]
    fn parses_memory_used() {
        let used = parse_memory_used(SAMPLE).unwrap();
        // 4801M + 5292M = 10093M = 10093 * 1<<20
        assert_eq!(used, (4801u64 + 5292u64) * (1u64 << 20));
    }

    #[test]
    fn parses_size_units() {
        assert_eq!(parse_size("1024K"), Some(1024 * 1024));
        assert_eq!(parse_size("2M"), Some(2 * 1024 * 1024));
        assert_eq!(parse_size("3.5G"), Some((3.5 * (1u64 << 30) as f64) as u64));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("nope"), None);
    }
}
