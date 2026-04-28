//! macOS system collectors. Each collector spawns a small child
//! process (top, sysctl, netstat, powermetrics) and parses its output.

use anyhow::Result;
use tokio::process::Command;
use tracing::warn;

use crate::snapshot::Snapshot;

mod cpu_mem;
mod network;
mod power;
mod sysctl;

/// Collect a complete snapshot, running the slow commands concurrently.
pub async fn collect(no_power: bool) -> Result<Snapshot> {
    let power_fut = async {
        if no_power {
            Ok(None)
        } else {
            power::cpu_power_watts().await
        }
    };

    let (power_res, cpu_mem_res, total_res, net_res) = tokio::join!(
        power_fut,
        cpu_mem::cpu_and_memory(),
        sysctl::memory_total_bytes(),
        network::interfaces(),
    );

    let (cpu_usage_ratio, memory_used_bytes) = match cpu_mem_res {
        Ok((c, m)) => (c, m),
        Err(e) => {
            warn!(collector = "top", error = %e, "collector failed");
            (None, None)
        }
    };

    Ok(Snapshot {
        cpu_power_watts: log_warn(power_res, "powermetrics").flatten(),
        cpu_usage_ratio,
        memory_used_bytes,
        memory_total_bytes: log_warn(total_res, "sysctl hw.memsize").flatten(),
        interfaces: log_warn(net_res, "netstat").unwrap_or_default(),
    })
}

fn log_warn<T>(res: Result<T>, what: &'static str) -> Option<T> {
    match res {
        Ok(v) => Some(v),
        Err(e) => {
            warn!(collector = what, error = %e, "collector failed");
            None
        }
    }
}

/// Helper: spawn `cmd args...`, return stdout as String. Errors on
/// non-zero exit or non-UTF-8 output.
pub(crate) async fn run_cmd(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program).args(args).output().await?;
    if !out.status.success() {
        anyhow::bail!(
            "{} {:?} exited {:?}: {}",
            program,
            args,
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}
