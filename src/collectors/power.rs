//! CPU package power, via `powermetrics --samplers cpu_power`.
//! Requires running as root (powermetrics needs the root entitlement).

use anyhow::Result;

use super::run_cmd;

pub async fn cpu_power_watts() -> Result<Option<f64>> {
    let out = run_cmd(
        "powermetrics",
        &["--samplers", "cpu_power", "-i", "500", "-n", "1"],
    )
    .await?;
    Ok(parse(&out))
}

fn parse(out: &str) -> Option<f64> {
    for line in out.lines() {
        let t = line.trim();
        // "CPU Power: 1234 mW"
        if let Some(rest) = t.strip_prefix("CPU Power:") {
            return parse_mw(rest);
        }
        // "Combined Power (CPU + GPU + ANE): 1234 mW"
        if t.starts_with("Combined Power") {
            if let Some((_, v)) = t.split_once(':') {
                return parse_mw(v);
            }
        }
    }
    None
}

/// Parse `"  1234.56 mW"` style values, returning watts.
fn parse_mw(s: &str) -> Option<f64> {
    let s = s.trim();
    let num: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if num.is_empty() {
        return None;
    }
    let n: f64 = num.parse().ok()?;
    Some(n / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_power_line() {
        let sample = "
*** Sampled system activity (Sun Apr 27 17:00:00 2026 ) ***

**** Processor usage ****

CPU Power: 4123 mW
GPU Power: 12 mW
";
        assert_eq!(parse(sample), Some(4.123));
    }

    #[test]
    fn parses_combined_power_line() {
        assert_eq!(
            parse("Combined Power (CPU + GPU + ANE): 5678 mW\n"),
            Some(5.678)
        );
    }

    #[test]
    fn returns_none_when_no_match() {
        assert_eq!(parse("nothing here\n"), None);
    }
}
