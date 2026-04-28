# dormlab-nodeexporter

Tiny Prometheus exporter for macOS Mac mini host metrics. Replaces the
original Python prototype (~700 MB RSS) with a Rust binary (~1 MB RSS).

## Metrics

| Metric | Source |
|--------|--------|
| `dormlab_cpu_power_watts` | `powermetrics --samplers cpu_power` (CPU package power) |
| `dormlab_cpu_usage_ratio` | `top -l 1` user + sys, 0..1 |
| `dormlab_memory_used_bytes` | `top` PhysMem: wired + compressor (excludes file-backed cache; matches Activity Monitor's "Memory Used" view) |
| `dormlab_memory_total_bytes` | `sysctl hw.memsize` |
| `dormlab_network_rx_bytes_total{iface}` | `netstat -ibn` Ibytes per non-loopback iface |
| `dormlab_network_tx_bytes_total{iface}` | `netstat -ibn` Obytes per non-loopback iface |

Listens on `0.0.0.0:9101` and serves `GET /metrics`.

## Build

```sh
cargo build --release --target aarch64-apple-darwin
```

Produces a ~370 KB binary at `target/aarch64-apple-darwin/release/dormlab-nodeexporter`.

## Deploy (per Mac mini)

```sh
sudo install -m 755 target/aarch64-apple-darwin/release/dormlab-nodeexporter \
  /usr/local/sbin/dormlab-nodeexporter
sudo install -m 644 dev.dormlab.nodeexporter.plist \
  /Library/LaunchDaemons/dev.dormlab.nodeexporter.plist
sudo launchctl bootstrap system /Library/LaunchDaemons/dev.dormlab.nodeexporter.plist
```

`powermetrics` requires root, hence the system-level LaunchDaemon.

## Why Rust?

The original Python prototype leaked memory under the 15-second Prometheus
scrape cadence — accumulated to ~700 MB RSS over hours due to threading
overhead and subprocess output buffering. A Rust port using only the
standard library settled at **1 MB RSS** with no GC required and a single
~370 KB binary instead of a Python interpreter.

## Why these specific metrics?

The colima VMs each have node-exporter inside, but those see only the VM's
8-CPU/12-GiB allocation — not the mini's full 10/16. They also can't get
power metrics. This exporter runs on the **host** to surface the metrics
node-exporter can't, then Prometheus (in-cluster) scrapes it via Tailscale.
