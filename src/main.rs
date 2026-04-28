//! Tiny Prometheus exporter for macOS Mac mini host metrics.
//!
//! Listens on 0.0.0.0:9101 and serves /metrics. Designed to run as
//! a launchd LaunchDaemon (root) so it can shell out to `powermetrics`.
//!
//! Exposes:
//!   - dormlab_cpu_power_watts        (powermetrics CPU package power)
//!   - dormlab_cpu_usage_ratio        (top CPU user+sys / 100)
//!   - dormlab_memory_used_bytes      (top: wired + compressor; matches
//!                                     Activity Monitor's headroom view,
//!                                     excludes file-backed cache)
//!   - dormlab_memory_total_bytes     (sysctl hw.memsize)
//!   - dormlab_network_rx_bytes_total (counter, per iface, from netstat)
//!   - dormlab_network_tx_bytes_total (counter, per iface, from netstat)
//!
//! No external dependencies. Standard library only. ~5 MB binary, ~5 MB RSS.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::Duration;

const PORT: u16 = 9101;
const READ_TIMEOUT_MS: u64 = 5_000;

fn run(args: &[&str]) -> Option<String> {
    Command::new(args[0])
        .args(&args[1..])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn cpu_power_watts() -> Option<f64> {
    // `powermetrics --samplers cpu_power -i 500 -n 1`
    let out = run(&["powermetrics", "--samplers", "cpu_power", "-i", "500", "-n", "1"])?;
    for line in out.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("CPU Power:") {
            return parse_mw(rest);
        }
        if t.starts_with("Combined Power") {
            if let Some(idx) = t.find(':') {
                return parse_mw(&t[idx + 1..]);
            }
        }
    }
    None
}

/// Parse a tail like "1234.56 mW" returning watts.
fn parse_mw(s: &str) -> Option<f64> {
    let s = s.trim();
    let num_end = s
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let n: f64 = s[..num_end].parse().ok()?;
    Some(n / 1000.0)
}

fn cpu_usage_ratio() -> Option<f64> {
    // `top -l 1 -n 0`
    let out = run(&["top", "-l", "1", "-n", "0"])?;
    for line in out.lines() {
        // "CPU usage: 5.55% user, 4.44% sys, 89.99% idle"
        if let Some(rest) = line.strip_prefix("CPU usage: ") {
            let mut user = None;
            let mut sys = None;
            for part in rest.split(',') {
                let part = part.trim();
                if let Some(num) = part.strip_suffix("% user") {
                    user = num.trim().parse::<f64>().ok();
                } else if let Some(num) = part.strip_suffix("% sys") {
                    sys = num.trim().parse::<f64>().ok();
                }
            }
            if let (Some(u), Some(s)) = (user, sys) {
                return Some((u + s) / 100.0);
            }
        }
    }
    None
}

fn memory_bytes() -> (Option<u64>, Option<u64>) {
    // total: sysctl hw.memsize
    let total: Option<u64> = run(&["sysctl", "-n", "hw.memsize"])
        .and_then(|s| s.trim().parse().ok());

    // used = wired + compressor, parsed from top's PhysMem line.
    // Excludes file-backed cache (top calls it "used" but it's reclaimable).
    let used: Option<u64> = run(&["top", "-l", "1", "-n", "0"]).and_then(|out| {
        for line in out.lines() {
            if let Some(rest) = line.strip_prefix("PhysMem:") {
                // "... (4801M wired, 5292M compressor), 91M unused."
                let open = rest.find('(')?;
                let close = rest.find(')')?;
                let inner = &rest[open + 1..close];
                let mut wired = None;
                let mut compr = None;
                for part in inner.split(',') {
                    let part = part.trim();
                    if let Some(n) = part.strip_suffix(" wired") {
                        wired = parse_size(n);
                    } else if let Some(n) = part.strip_suffix(" compressor") {
                        compr = parse_size(n);
                    }
                }
                return wired.zip(compr).map(|(w, c)| w + c);
            }
        }
        None
    });
    (used, total)
}

/// Parse strings like "4801M" or "5.2G" into bytes.
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, unit) = s.split_at(s.len() - 1);
    let n: f64 = num.parse().ok()?;
    let mult = match unit {
        "K" => 1024u64,
        "M" => 1024u64 * 1024,
        "G" => 1024u64 * 1024 * 1024,
        "T" => 1024u64 * 1024 * 1024 * 1024,
        _ => return None,
    };
    Some((n * mult as f64) as u64)
}

/// Returns vec of (iface, rx_bytes, tx_bytes), one entry per non-loopback interface.
fn network_bytes() -> Vec<(String, u64, u64)> {
    let Some(out) = run(&["netstat", "-ibn"]) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for line in out.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 11 {
            continue;
        }
        let iface = f[0];
        if iface == "lo0" || !seen.insert(iface.to_string()) {
            continue;
        }
        // Format: Name Mtu Network Address Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll
        // f[6] = Ibytes (rx), f[9] = Obytes (tx).
        let Ok(rx) = f[6].parse::<u64>() else { continue };
        let Ok(tx) = f[9].parse::<u64>() else { continue };
        result.push((iface.to_string(), rx, tx));
    }
    result
}

fn build_metrics() -> String {
    let mut out = String::with_capacity(2048);

    if let Some(p) = cpu_power_watts() {
        out.push_str("# HELP dormlab_cpu_power_watts CPU package power in watts\n");
        out.push_str("# TYPE dormlab_cpu_power_watts gauge\n");
        out.push_str(&format!("dormlab_cpu_power_watts {p}\n"));
    }

    if let Some(c) = cpu_usage_ratio() {
        out.push_str("# HELP dormlab_cpu_usage_ratio CPU usage ratio (user + sys), 0..1\n");
        out.push_str("# TYPE dormlab_cpu_usage_ratio gauge\n");
        out.push_str(&format!("dormlab_cpu_usage_ratio {c}\n"));
    }

    let (used, total) = memory_bytes();
    if let Some(u) = used {
        out.push_str("# HELP dormlab_memory_used_bytes Memory in use (wired + compressor)\n");
        out.push_str("# TYPE dormlab_memory_used_bytes gauge\n");
        out.push_str(&format!("dormlab_memory_used_bytes {u}\n"));
    }
    if let Some(t) = total {
        out.push_str("# HELP dormlab_memory_total_bytes Physical memory total\n");
        out.push_str("# TYPE dormlab_memory_total_bytes gauge\n");
        out.push_str(&format!("dormlab_memory_total_bytes {t}\n"));
    }

    let net = network_bytes();
    if !net.is_empty() {
        out.push_str("# HELP dormlab_network_rx_bytes_total Per-iface RX bytes\n");
        out.push_str("# TYPE dormlab_network_rx_bytes_total counter\n");
        for (iface, rx, _) in &net {
            out.push_str(&format!(
                "dormlab_network_rx_bytes_total{{iface=\"{iface}\"}} {rx}\n"
            ));
        }
        out.push_str("# HELP dormlab_network_tx_bytes_total Per-iface TX bytes\n");
        out.push_str("# TYPE dormlab_network_tx_bytes_total counter\n");
        for (iface, _, tx) in &net {
            out.push_str(&format!(
                "dormlab_network_tx_bytes_total{{iface=\"{iface}\"}} {tx}\n"
            ));
        }
    }

    out
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))?;
    stream.set_write_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))?;

    // Read just enough of the request to find the path.
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let _method = parts.next();
    let path = parts.next().unwrap_or("").to_string();
    // Drain headers so the client doesn't see RST.
    let mut header = String::new();
    while reader.read_line(&mut header)? > 2 {
        header.clear();
    }
    drop(reader);

    if path != "/metrics" {
        let body = b"not found\n";
        write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        stream.write_all(body)?;
        return Ok(());
    }

    let body = build_metrics();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())?;
    Ok(())
}

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", PORT))?;
    eprintln!("dormlab-nodeexporter listening on 0.0.0.0:{PORT}");
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                if let Err(e) = handle(s) {
                    eprintln!("request error: {e}");
                }
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    Ok(())
}
