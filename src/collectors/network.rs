//! Per-interface byte counters from `netstat -ibn`.
//!
//! `netstat -ibn` columns:
//!   Name Mtu Network Address Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll
//!         0   1     2      3     4     5      6     7     8      9    10
//!
//! Some interfaces appear multiple times (one per address family). We
//! keep the first row per name and skip loopback.

use anyhow::Result;
use std::collections::HashSet;

use super::run_cmd;
use crate::snapshot::IfaceCounters;

pub async fn interfaces() -> Result<Vec<IfaceCounters>> {
    let out = run_cmd("netstat", &["-ibn"]).await?;
    Ok(parse(&out))
}

fn parse(out: &str) -> Vec<IfaceCounters> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for line in out.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 11 {
            continue;
        }
        let name = f[0];
        if name == "lo0" || !seen.insert(name.to_string()) {
            continue;
        }
        let Ok(rx) = f[6].parse::<u64>() else {
            continue;
        };
        let Ok(tx) = f[9].parse::<u64>() else {
            continue;
        };
        result.push(IfaceCounters {
            name: name.to_string(),
            rx_bytes: rx,
            tx_bytes: tx,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Name  Mtu   Network       Address            Ipkts Ierrs     Ibytes    Opkts Oerrs     Obytes  Coll
lo0   16384 <Link#1>                       1234       0     567890     1234       0     567890     0
en0   1500  <Link#5>      a8:97:b8:00:00:00 1000       0   12345678      900       0    9876543     0
en0   1500  192.168/24    192.168.1.10      1000       0   12345678      900       0    9876543     0
en2   1500  <Link#7>      36:5a:7a:00:00:00 555        0    1111111      444       0    2222222     0
";

    #[test]
    fn parses_unique_interfaces() {
        let v = parse(SAMPLE);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "en0");
        assert_eq!(v[0].rx_bytes, 12345678);
        assert_eq!(v[0].tx_bytes, 9876543);
        assert_eq!(v[1].name, "en2");
    }

    #[test]
    fn skips_lo0() {
        let v = parse(SAMPLE);
        assert!(v.iter().all(|i| i.name != "lo0"));
    }
}
