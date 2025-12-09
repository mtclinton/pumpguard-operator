//! Prometheus metrics service for PumpGuard

use prometheus::{
    Counter, CounterVec, Encoder, Gauge, GaugeVec, Histogram, HistogramOpts, HistogramVec, Opts,
    Registry, TextEncoder,
};
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

/// Metrics service for Prometheus
pub struct MetricsService {
    registry: Registry,
    start_time: Instant,

    // Token Monitor metrics
    pub tokens_detected: Counter,
    pub token_alerts: Counter,
    pub pending_tokens: Gauge,

    // Rug Detector metrics
    pub tokens_watched: Gauge,
    pub rugs_detected: CounterVec,
    pub suspicious_activity: CounterVec,
    pub suspicion_score: GaugeVec,

    // Whale Watcher metrics
    pub whales_tracked: Gauge,
    pub whale_transactions: CounterVec,
    pub whale_volume: CounterVec,
    pub accumulation_alerts: Counter,
    pub dump_alerts: Counter,

    // System metrics
    pub alerts_sent: CounterVec,
    pub rpc_requests: CounterVec,
    pub rpc_latency: HistogramVec,
    pub websocket_connected: Gauge,
    pub module_status: GaugeVec,
    pub uptime: Gauge,
}

impl MetricsService {
    /// Create a new metrics service
    pub fn new() -> Self {
        let registry = Registry::new();

        // Token Monitor metrics
        let tokens_detected = Counter::new("pumpguard_tokens_detected_total", "Total tokens detected")
            .unwrap();
        let token_alerts = Counter::new("pumpguard_token_alerts_total", "Total token alerts sent")
            .unwrap();
        let pending_tokens = Gauge::new("pumpguard_tokens_tracked", "Tokens being tracked")
            .unwrap();

        // Rug Detector metrics
        let tokens_watched = Gauge::new("pumpguard_tokens_watched", "Tokens watched for rugs")
            .unwrap();
        let rugs_detected = CounterVec::new(
            Opts::new("pumpguard_rugs_detected_total", "Rug pulls detected"),
            &["severity"],
        )
        .unwrap();
        let suspicious_activity = CounterVec::new(
            Opts::new("pumpguard_suspicious_activity_total", "Suspicious activities"),
            &["type"],
        )
        .unwrap();
        let suspicion_score = GaugeVec::new(
            Opts::new("pumpguard_token_suspicion_score", "Per-token suspicion score"),
            &["mint", "symbol"],
        )
        .unwrap();

        // Whale Watcher metrics
        let whales_tracked = Gauge::new("pumpguard_whales_tracked", "Whale wallets tracked")
            .unwrap();
        let whale_transactions = CounterVec::new(
            Opts::new("pumpguard_whale_transactions_total", "Whale transactions"),
            &["type"],
        )
        .unwrap();
        let whale_volume = CounterVec::new(
            Opts::new("pumpguard_whale_volume_sol_total", "Whale volume in SOL"),
            &["type"],
        )
        .unwrap();
        let accumulation_alerts = Counter::new(
            "pumpguard_accumulation_alerts_total",
            "Accumulation pattern alerts",
        )
        .unwrap();
        let dump_alerts = Counter::new("pumpguard_dump_alerts_total", "Dump pattern alerts")
            .unwrap();

        // System metrics
        let alerts_sent = CounterVec::new(
            Opts::new("pumpguard_alerts_sent_total", "Total alerts sent"),
            &["type", "channel"],
        )
        .unwrap();
        let rpc_requests = CounterVec::new(
            Opts::new("pumpguard_rpc_requests_total", "RPC requests"),
            &["method", "status"],
        )
        .unwrap();
        let rpc_latency = HistogramVec::new(
            HistogramOpts::new("pumpguard_rpc_latency_seconds", "RPC latency")
                .buckets(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5]),
            &["method"],
        )
        .unwrap();
        let websocket_connected = Gauge::new(
            "pumpguard_websocket_connected",
            "WebSocket connection status",
        )
        .unwrap();
        let module_status = GaugeVec::new(
            Opts::new("pumpguard_module_running", "Module status"),
            &["module"],
        )
        .unwrap();
        let uptime = Gauge::new("pumpguard_uptime_seconds", "Application uptime").unwrap();

        // Register all metrics
        registry.register(Box::new(tokens_detected.clone())).unwrap();
        registry.register(Box::new(token_alerts.clone())).unwrap();
        registry.register(Box::new(pending_tokens.clone())).unwrap();
        registry.register(Box::new(tokens_watched.clone())).unwrap();
        registry.register(Box::new(rugs_detected.clone())).unwrap();
        registry.register(Box::new(suspicious_activity.clone())).unwrap();
        registry.register(Box::new(suspicion_score.clone())).unwrap();
        registry.register(Box::new(whales_tracked.clone())).unwrap();
        registry.register(Box::new(whale_transactions.clone())).unwrap();
        registry.register(Box::new(whale_volume.clone())).unwrap();
        registry.register(Box::new(accumulation_alerts.clone())).unwrap();
        registry.register(Box::new(dump_alerts.clone())).unwrap();
        registry.register(Box::new(alerts_sent.clone())).unwrap();
        registry.register(Box::new(rpc_requests.clone())).unwrap();
        registry.register(Box::new(rpc_latency.clone())).unwrap();
        registry.register(Box::new(websocket_connected.clone())).unwrap();
        registry.register(Box::new(module_status.clone())).unwrap();
        registry.register(Box::new(uptime.clone())).unwrap();

        info!(target: "METRICS", "Prometheus metrics initialized");

        Self {
            registry,
            start_time: Instant::now(),
            tokens_detected,
            token_alerts,
            pending_tokens,
            tokens_watched,
            rugs_detected,
            suspicious_activity,
            suspicion_score,
            whales_tracked,
            whale_transactions,
            whale_volume,
            accumulation_alerts,
            dump_alerts,
            alerts_sent,
            rpc_requests,
            rpc_latency,
            websocket_connected,
            module_status,
            uptime,
        }
    }

    /// Record token detected
    pub fn record_token_detected(&self) {
        self.tokens_detected.inc();
    }

    /// Record rug detected
    pub fn record_rug_detected(&self, severity: &str) {
        self.rugs_detected.with_label_values(&[severity]).inc();
    }

    /// Record whale transaction
    pub fn record_whale_transaction(&self, tx_type: &str, volume_sol: f64) {
        self.whale_transactions.with_label_values(&[tx_type]).inc();
        self.whale_volume
            .with_label_values(&[tx_type])
            .inc_by(volume_sol);
    }

    /// Set module status
    pub fn set_module_status(&self, module: &str, running: bool) {
        self.module_status
            .with_label_values(&[module])
            .set(if running { 1.0 } else { 0.0 });
    }

    /// Get metrics as Prometheus text format
    pub fn get_metrics(&self) -> String {
        // Update uptime
        self.uptime.set(self.start_time.elapsed().as_secs_f64());

        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }
}

impl Default for MetricsService {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for MetricsService {
    fn clone(&self) -> Self {
        // Return a new instance that shares the same metrics
        // This is safe because Prometheus metrics are thread-safe
        Self {
            registry: self.registry.clone(),
            start_time: self.start_time,
            tokens_detected: self.tokens_detected.clone(),
            token_alerts: self.token_alerts.clone(),
            pending_tokens: self.pending_tokens.clone(),
            tokens_watched: self.tokens_watched.clone(),
            rugs_detected: self.rugs_detected.clone(),
            suspicious_activity: self.suspicious_activity.clone(),
            suspicion_score: self.suspicion_score.clone(),
            whales_tracked: self.whales_tracked.clone(),
            whale_transactions: self.whale_transactions.clone(),
            whale_volume: self.whale_volume.clone(),
            accumulation_alerts: self.accumulation_alerts.clone(),
            dump_alerts: self.dump_alerts.clone(),
            alerts_sent: self.alerts_sent.clone(),
            rpc_requests: self.rpc_requests.clone(),
            rpc_latency: self.rpc_latency.clone(),
            websocket_connected: self.websocket_connected.clone(),
            module_status: self.module_status.clone(),
            uptime: self.uptime.clone(),
        }
    }
}


