# dormlab-nodeexporter

A small Prometheus exporter for **macOS** hosts. Exposes the metrics that
the official Prometheus `node_exporter` and Apple's tooling don't make
easy to surface together — CPU package power, CPU usage, real "memory
used" (excluding reclaimable cache), and per-interface byte counters —
over a single endpoint.

Built for headless Mac mini server racks; works on any macOS host.

## Metrics

| Name | Type | Source | Notes |
|------|------|--------|-------|
| `dormlab_cpu_power_watts` | gauge | `powermetrics --samplers cpu_power` | Requires running as root. Set `--no-power` to skip. |
| `dormlab_cpu_usage_ratio` | gauge | `top -l 1 -n 0` (`user + sys`) | 0..1 |
| `dormlab_memory_used_bytes` | gauge | `top` PhysMem (`wired + compressor`) | Excludes reclaimable file-backed cache. Matches Activity Monitor's headroom view. |
| `dormlab_memory_total_bytes` | gauge | `sysctl hw.memsize` | |
| `dormlab_network_rx_bytes_total{iface}` | counter | `netstat -ibn` `Ibytes` | Loopback skipped, deduplicated per iface. |
| `dormlab_network_tx_bytes_total{iface}` | counter | `netstat -ibn` `Obytes` | Same. |
| `dormlab_nodeexporter_up{version}` | gauge | always `1` | Liveness for `up{}` queries. |

## Architecture

A background tokio task runs all four shell-out collectors concurrently
(`tokio::join!`) every `--refresh-secs` seconds and updates a shared
Prometheus `Registry`. The `axum` HTTP handler reads from the registry —
no subprocesses run inside a request, so the `/metrics` scrape is fast and
predictable even when `powermetrics` takes ~500 ms to sample.

Counters stay honest across rapid scrapes by tracking the previous
absolute kernel value per interface and feeding `inc_by(delta)` to
Prometheus, so `rate()` queries work normally and counter resets (iface
reload, host reboot) are absorbed via `saturating_sub`.

## Endpoints

- `GET /metrics` — Prometheus text exposition.
- `GET /healthz` — `200 ok` if the process is alive.
- `GET /` — landing page with links to the above.

## Build

```sh
cargo build --release --target aarch64-apple-darwin
```

Produces a single ~1.8 MB binary at
`target/aarch64-apple-darwin/release/dormlab-nodeexporter`. No runtime
dependencies — it shells out to `powermetrics`, `top`, `sysctl`, and
`netstat`, all bundled with macOS.

```sh
cargo test --release --target aarch64-apple-darwin
```

## CLI

```text
Usage: dormlab-nodeexporter [OPTIONS]

Options:
      --bind <BIND>                   [env: DORMLAB_BIND=] [default: 0.0.0.0:9101]
      --refresh-secs <REFRESH_SECS>   [env: DORMLAB_REFRESH_SECS=] [default: 5]
      --no-power                      [env: DORMLAB_NO_POWER=]
  -h, --help                          Print help
  -V, --version                       Print version
```

Set `RUST_LOG=dormlab_nodeexporter=debug` for verbose logs.

## Deploy as a LaunchDaemon

`powermetrics` requires root. Drop the included
[`dev.dormlab.nodeexporter.plist`](./dev.dormlab.nodeexporter.plist) into
`/Library/LaunchDaemons/` and bootstrap it:

```sh
sudo install -m 755 target/aarch64-apple-darwin/release/dormlab-nodeexporter \
  /usr/local/sbin/dormlab-nodeexporter
sudo install -m 644 dev.dormlab.nodeexporter.plist \
  /Library/LaunchDaemons/dev.dormlab.nodeexporter.plist
sudo launchctl bootstrap system /Library/LaunchDaemons/dev.dormlab.nodeexporter.plist
```

Logs go to `/var/log/dormlab-nodeexporter.log`.

## Prometheus scrape config

```yaml
scrape_configs:
  - job_name: macos-host
    static_configs:
      - targets:
          - mac-1.example.com:9101
          - mac-2.example.com:9101
```

The exporter listens on `0.0.0.0:9101` by default. Restrict at the
firewall or with `--bind 127.0.0.1:9101` plus a Tailscale / SSH tunnel
if you care.

## Why not the upstream `node_exporter`?

The Prometheus project's `node_exporter` works on macOS via a Darwin
collector, but it's missing power metrics (no `powermetrics` integration)
and its memory accounting reports the kernel's "active" pages rather than
the user-meaningful "wired + compressor" view. This exporter intentionally
covers a smaller surface — just the four things you usually want from a
macOS host — and keeps the binary tiny.

## License

MIT — see [LICENSE](./LICENSE).
