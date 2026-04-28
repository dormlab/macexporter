//! dormlab-nodeexporter
//!
//! A Prometheus node exporter for macOS hosts. Exposes:
//!
//! - `dormlab_cpu_power_watts` — CPU package power in watts
//!   (from `powermetrics --samplers cpu_power`; requires running as root).
//! - `dormlab_cpu_usage_ratio` — fraction of CPU spent in user + sys, 0..1.
//! - `dormlab_memory_used_bytes` — wired + compressed, parsed from `top`'s
//!   `PhysMem` line. Excludes file-backed cache (which `top` calls "used"
//!   but is reclaimable), so this matches the Activity Monitor "Memory
//!   Used" view that's relevant for headroom.
//! - `dormlab_memory_total_bytes` — `sysctl hw.memsize`.
//! - `dormlab_network_rx_bytes_total{iface=}` /
//!   `dormlab_network_tx_bytes_total{iface=}` — per-interface byte counters
//!   from `netstat -ibn`. Loopback excluded.
//!
//! Architecture: a background task refreshes a shared snapshot every
//! `--refresh-interval` seconds by spawning the macOS tools concurrently
//! (`tokio::join!`). The HTTP handler reads the snapshot under an
//! `RwLock` and serves Prometheus text from a registry — request handling
//! never blocks on a subprocess.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use clap::Parser;
use prometheus::{Encoder, Registry, TextEncoder};
use tokio::signal;
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

mod collectors;
mod metrics;
mod snapshot;

use metrics::Metrics;
use snapshot::Snapshot;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Address to bind the HTTP server to.
    #[arg(long, env = "DORMLAB_BIND", default_value = "0.0.0.0:9101")]
    bind: SocketAddr,

    /// How often to refresh the metrics snapshot, in seconds.
    #[arg(long, env = "DORMLAB_REFRESH_SECS", default_value_t = 5)]
    refresh_secs: u64,

    /// Skip the powermetrics collector. Useful when not running as root.
    #[arg(long, env = "DORMLAB_NO_POWER", default_value_t = false)]
    no_power: bool,
}

#[derive(Clone)]
struct AppState {
    registry: Arc<Registry>,
}

#[allow(dead_code)]
struct Backend {
    metrics: Arc<Metrics>,
    snapshot: Arc<RwLock<Snapshot>>,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dormlab_nodeexporter=info,tower_http=warn".into()),
        )
        .with_target(false)
        .compact()
        .init();

    let args = Args::parse();
    info!(
        bind = %args.bind,
        refresh_secs = args.refresh_secs,
        powermetrics = !args.no_power,
        "starting dormlab-nodeexporter"
    );

    let registry = Arc::new(Registry::new());
    let metrics = Arc::new(Metrics::new(&registry).context("registering Prometheus metrics")?);
    let snapshot = Arc::new(RwLock::new(Snapshot::default()));

    // Initial collection so /metrics is useful immediately.
    refresh_once(&snapshot, &metrics, args.no_power).await;

    let state = AppState {
        registry: registry.clone(),
    };

    // Background snapshot refresher.
    {
        let snapshot = snapshot.clone();
        let metrics = metrics.clone();
        let interval = Duration::from_secs(args.refresh_secs.max(1));
        let no_power = args.no_power;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Skip the first tick — we just did one.
            tick.tick().await;
            loop {
                tick.tick().await;
                refresh_once(&snapshot, &metrics, no_power).await;
            }
        });
    }

    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics_handler))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    info!(addr = %listener.local_addr()?, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("axum serve failed")?;

    Ok(())
}

async fn refresh_once(snapshot: &Arc<RwLock<Snapshot>>, metrics: &Arc<Metrics>, no_power: bool) {
    match collectors::collect(no_power).await {
        Ok(snap) => {
            metrics.observe(&snap);
            *snapshot.write().await = snap;
        }
        Err(err) => warn!(error = %err, "snapshot refresh failed"),
    }
}

async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let mut buf = Vec::with_capacity(4096);
    let encoder = TextEncoder::new();
    let metric_families = state.registry.gather();
    if let Err(e) = encoder.encode(&metric_families, &mut buf) {
        error!(error = %e, "failed to encode metrics");
        return (StatusCode::INTERNAL_SERVER_ERROR, "encode failure").into_response();
    }
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        buf,
    )
        .into_response()
}

async fn index() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        concat!(
            "<!doctype html><html><body>",
            "<h1>dormlab-nodeexporter</h1>",
            "<ul><li><a href=\"/metrics\">/metrics</a></li>",
            "<li><a href=\"/healthz\">/healthz</a></li></ul>",
            "</body></html>",
        ),
    )
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok\n")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            s.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("ctrl-c, shutting down"),
        _ = terminate => info!("SIGTERM, shutting down"),
    }
}
