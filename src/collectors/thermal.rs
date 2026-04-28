//! Thermal pressure on macOS, via `powermetrics --samplers thermal`.
//!
//! Apple Silicon doesn't expose CPU temperature in degrees through any
//! public CLI — `powermetrics --samplers smc` is Intel-only, and real
//! per-sensor temperatures require the private IOReport framework
//! (what `mactop` / `asitop` use). What we *do* get is a categorical
//! "thermal pressure" level: Nominal / Fair / Serious / Critical. We
//! emit it as a small integer gauge so it can drive alerts and color
//! thresholds.
//!
//! Mapping:
//! - 0 → Nominal  (system is comfortably below thermal limits)
//! - 1 → Fair     (some throttling possible soon)
//! - 2 → Serious  (active throttling)
//! - 3 → Critical (severe throttling, immediate action recommended)
//!
//! Requires running as root (powermetrics needs the root entitlement).

use anyhow::Result;

use super::run_cmd;

pub async fn pressure_level() -> Result<Option<u8>> {
    let out = run_cmd(
        "powermetrics",
        &["--samplers", "thermal", "-i", "500", "-n", "1"],
    )
    .await?;
    Ok(parse(&out))
}

fn parse(out: &str) -> Option<u8> {
    for line in out.lines() {
        let t = line.trim();
        // "Current pressure level: Nominal"
        if let Some(rest) = t.strip_prefix("Current pressure level:") {
            return match rest.trim() {
                "Nominal" => Some(0),
                "Fair" => Some(1),
                "Serious" => Some(2),
                "Critical" => Some(3),
                _ => None,
            };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_levels() {
        assert_eq!(parse("Current pressure level: Nominal\n"), Some(0));
        assert_eq!(parse("Current pressure level: Fair\n"), Some(1));
        assert_eq!(parse("Current pressure level: Serious\n"), Some(2));
        assert_eq!(parse("Current pressure level: Critical\n"), Some(3));
    }

    #[test]
    fn parses_with_surrounding_output() {
        let sample = "
*** Thermal pressure ***

Current pressure level: Nominal
";
        assert_eq!(parse(sample), Some(0));
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(parse("Current pressure level: Glorious\n"), None);
        assert_eq!(parse("nothing here\n"), None);
    }
}
