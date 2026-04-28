//! Prometheus metric registration + ingestion of [`Snapshot`] values.

use anyhow::Result;
use prometheus::{Gauge, GaugeVec, IntCounterVec, IntGauge, Opts, Registry};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::snapshot::Snapshot;

pub struct Metrics {
    cpu_power_watts: Gauge,
    cpu_usage_ratio: Gauge,
    memory_used_bytes: IntGauge,
    memory_total_bytes: IntGauge,

    rx_bytes_total: IntCounterVec,
    tx_bytes_total: IntCounterVec,

    /// Prometheus IntCounters can only `inc_by(n)`, not `set(n)`. We track
    /// the last raw kernel value per iface so we can convert absolute
    /// counters into deltas.
    last_rx: Mutex<HashMap<String, u64>>,
    last_tx: Mutex<HashMap<String, u64>>,

    /// Always-on liveness gauge so `up{}` is meaningful.
    up: GaugeVec,
}

impl Metrics {
    pub fn new(registry: &Registry) -> Result<Self> {
        let cpu_power_watts = Gauge::with_opts(Opts::new(
            "dormlab_cpu_power_watts",
            "CPU package power, in watts (powermetrics).",
        ))?;
        let cpu_usage_ratio = Gauge::with_opts(Opts::new(
            "dormlab_cpu_usage_ratio",
            "Fraction of CPU spent in user + sys, in [0, 1] (top).",
        ))?;
        let memory_used_bytes = IntGauge::with_opts(Opts::new(
            "dormlab_memory_used_bytes",
            "Memory in use: wired + compressor (top PhysMem). Excludes \
             reclaimable file-backed cache.",
        ))?;
        let memory_total_bytes = IntGauge::with_opts(Opts::new(
            "dormlab_memory_total_bytes",
            "Physical memory installed (sysctl hw.memsize).",
        ))?;
        let rx_bytes_total = IntCounterVec::new(
            Opts::new(
                "dormlab_network_rx_bytes_total",
                "Bytes received per network interface (netstat -ibn).",
            ),
            &["iface"],
        )?;
        let tx_bytes_total = IntCounterVec::new(
            Opts::new(
                "dormlab_network_tx_bytes_total",
                "Bytes transmitted per network interface (netstat -ibn).",
            ),
            &["iface"],
        )?;
        let up = GaugeVec::new(
            Opts::new(
                "dormlab_nodeexporter_up",
                "1 if the exporter is alive (always emits 1).",
            ),
            &["version"],
        )?;
        up.with_label_values(&[env!("CARGO_PKG_VERSION")]).set(1.0);

        registry.register(Box::new(cpu_power_watts.clone()))?;
        registry.register(Box::new(cpu_usage_ratio.clone()))?;
        registry.register(Box::new(memory_used_bytes.clone()))?;
        registry.register(Box::new(memory_total_bytes.clone()))?;
        registry.register(Box::new(rx_bytes_total.clone()))?;
        registry.register(Box::new(tx_bytes_total.clone()))?;
        registry.register(Box::new(up.clone()))?;

        Ok(Self {
            cpu_power_watts,
            cpu_usage_ratio,
            memory_used_bytes,
            memory_total_bytes,
            rx_bytes_total,
            tx_bytes_total,
            last_rx: Mutex::new(HashMap::new()),
            last_tx: Mutex::new(HashMap::new()),
            up,
        })
    }

    /// Apply the values from a freshly collected [`Snapshot`].
    pub fn observe(&self, snap: &Snapshot) {
        if let Some(p) = snap.cpu_power_watts {
            self.cpu_power_watts.set(p);
        }
        if let Some(r) = snap.cpu_usage_ratio {
            self.cpu_usage_ratio.set(r);
        }
        if let Some(used) = snap.memory_used_bytes {
            self.memory_used_bytes.set(used as i64);
        }
        if let Some(total) = snap.memory_total_bytes {
            self.memory_total_bytes.set(total as i64);
        }

        // Counters: inc_by delta vs the last sample, handling kernel
        // counter resets (i.e. iface reload, reboot) by treating any
        // backwards motion as a fresh start.
        let mut last_rx = self.last_rx.lock().expect("rx mutex poisoned");
        let mut last_tx = self.last_tx.lock().expect("tx mutex poisoned");
        for iface in &snap.interfaces {
            let prev_rx = last_rx.get(&iface.name).copied().unwrap_or(0);
            let delta_rx = iface.rx_bytes.saturating_sub(prev_rx);
            if delta_rx > 0 {
                self.rx_bytes_total
                    .with_label_values(&[iface.name.as_str()])
                    .inc_by(delta_rx);
            }
            last_rx.insert(iface.name.clone(), iface.rx_bytes);

            let prev_tx = last_tx.get(&iface.name).copied().unwrap_or(0);
            let delta_tx = iface.tx_bytes.saturating_sub(prev_tx);
            if delta_tx > 0 {
                self.tx_bytes_total
                    .with_label_values(&[iface.name.as_str()])
                    .inc_by(delta_tx);
            }
            last_tx.insert(iface.name.clone(), iface.tx_bytes);
        }

        // Liveness — always emits 1 once registered.
        self.up
            .with_label_values(&[env!("CARGO_PKG_VERSION")])
            .set(1.0);
    }
}
