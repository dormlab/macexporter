//! Plain data structures for a single metrics snapshot.

#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    pub cpu_power_watts: Option<f64>,
    pub cpu_usage_ratio: Option<f64>,
    pub memory_used_bytes: Option<u64>,
    pub memory_total_bytes: Option<u64>,
    pub interfaces: Vec<IfaceCounters>,
}

#[derive(Debug, Clone)]
pub struct IfaceCounters {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}
