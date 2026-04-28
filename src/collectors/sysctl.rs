//! Static system info via `sysctl`.

use anyhow::Result;

use super::run_cmd;

pub async fn memory_total_bytes() -> Result<Option<u64>> {
    let out = run_cmd("sysctl", &["-n", "hw.memsize"]).await?;
    Ok(out.trim().parse().ok())
}
