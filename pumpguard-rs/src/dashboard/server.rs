//! HTTP API and WebSocket dashboard server

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::{error, info};

use crate::config::Config;
use crate::modules::{RugDetector, TokenMonitor, WhaleWatcher};
use crate::utils::{AlertService, DatabaseService, MetricsService};
use crate::utils::alerts::Alert;

/// Query params for list endpoints
#[derive(Debug, Deserialize)]
pub struct ListParams {
    limit: Option<usize>,
}

/// Filter request body
#[derive(Debug, Deserialize)]
pub struct FilterRequest {
    key: String,
    value: f64,
}

/// Blacklist/Whitelist request body
#[derive(Debug, Deserialize)]
pub struct AddressRequest {
    address: String,
}

/// Watch wallet request body
#[derive(Debug, Deserialize)]
pub struct WatchWalletRequest {
    address: String,
    label: Option<String>,
}

/// Watch token request body
#[derive(Debug, Deserialize)]
pub struct WatchTokenRequest {
    mint: String,
    name: String,
    symbol: String,
    creator: String,
    initial_liquidity: f64,
}

/// API success response
#[derive(Debug, Serialize)]
pub struct ApiResponse {
    success: bool,
    message: String,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    status: String,
    uptime: f64,
    modules: ModuleStatus,
}

#[derive(Debug, Serialize)]
pub struct ModuleStatus {
    token_monitor: bool,
    rug_detector: bool,
    whale_watcher: bool,
}

/// Stats response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsResponse {
    token_monitor: crate::modules::token_monitor::TokenMonitorStats,
    rug_detector: crate::modules::rug_detector::RugDetectorStats,
    whale_watcher: crate::modules::whale_watcher::WhaleWatcherStats,
}

/// WebSocket message types
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum WsMessage {
    #[serde(rename = "init")]
    Init {
        stats: StatsResponse,
        recent_alerts: Vec<Alert>,
    },
    #[serde(rename = "alert")]
    Alert(Alert),
    #[serde(rename = "stats")]
    Stats(StatsResponse),
}

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub token_monitor: TokenMonitor,
    pub rug_detector: RugDetector,
    pub whale_watcher: WhaleWatcher,
    pub alerts: Arc<AlertService>,
    pub database: Arc<DatabaseService>,
    pub metrics: Arc<MetricsService>,
    pub start_time: std::time::Instant,
}

/// Dashboard server
pub struct DashboardServer {
    config: Config,
    state: AppState,
}

impl DashboardServer {
    /// Create a new dashboard server
    pub fn new(
        config: Config,
        token_monitor: TokenMonitor,
        rug_detector: RugDetector,
        whale_watcher: WhaleWatcher,
        alerts: Arc<AlertService>,
        database: Arc<DatabaseService>,
        metrics: Arc<MetricsService>,
    ) -> Self {
        let state = AppState {
            config: config.clone(),
            token_monitor,
            rug_detector,
            whale_watcher,
            alerts,
            database,
            metrics,
            start_time: std::time::Instant::now(),
        };

        Self { config, state }
    }

    /// Start the dashboard server
    pub async fn start(&self) -> anyhow::Result<()> {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        let app = Router::new()
            // API Routes
            .route("/api/stats", get(get_stats))
            // Token Monitor
            .route("/api/tokens/recent", get(get_recent_tokens))
            .route("/api/tokens/start", post(start_token_monitor))
            .route("/api/tokens/stop", post(stop_token_monitor))
            .route("/api/tokens/filter", post(set_token_filter))
            .route("/api/tokens/blacklist", post(blacklist_creator))
            // Rug Detector
            .route("/api/rug/watched", get(get_watched_tokens))
            .route("/api/rug/token/:mint", get(get_token_details))
            .route("/api/rug/watch", post(watch_token))
            .route("/api/rug/start", post(start_rug_detector))
            .route("/api/rug/stop", post(stop_rug_detector))
            // Whale Watcher
            .route("/api/whales", get(get_whales))
            .route("/api/whales/movers", get(get_top_movers))
            .route("/api/whales/wallet/:address", get(get_wallet_activity))
            .route("/api/whales/watch", post(watch_wallet))
            .route("/api/whales/start", post(start_whale_watcher))
            .route("/api/whales/stop", post(stop_whale_watcher))
            // Alerts
            .route("/api/alerts", get(get_alerts))
            // Tokens from database
            .route("/api/tokens", get(get_db_tokens))
            .route("/api/tokens/:mint", get(get_db_token))
            // Prometheus metrics
            .route("/metrics", get(get_metrics))
            // Health checks
            .route("/health", get(health_check))
            .route("/ready", get(readiness_check))
            // WebSocket
            .route("/ws", get(ws_handler))
            // Static files (dashboard)
            .nest_service("/", ServeDir::new("public").fallback(get(serve_index)))
            .layer(cors)
            .with_state(self.state.clone());

        let addr = SocketAddr::from(([0, 0, 0, 0], self.config.dashboard_port));
        info!(target: "DASHBOARD", "✅ Dashboard running at http://localhost:{}", self.config.dashboard_port);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

// ============================================
// HANDLERS
// ============================================

async fn serve_index() -> Html<&'static str> {
    Html(include_str!("../../public/index.html"))
}

async fn get_stats(State(state): State<AppState>) -> Json<StatsResponse> {
    Json(StatsResponse {
        token_monitor: state.token_monitor.get_stats(),
        rug_detector: state.rug_detector.get_stats(),
        whale_watcher: state.whale_watcher.get_stats(),
    })
}

// Token Monitor handlers
async fn get_recent_tokens(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Json<Vec<crate::modules::token_monitor::DetectedToken>> {
    let limit = params.limit.unwrap_or(50);
    Json(state.token_monitor.get_recent_tokens(limit))
}

async fn start_token_monitor(State(state): State<AppState>) -> Json<ApiResponse> {
    let _ = state.token_monitor.start().await;
    Json(ApiResponse {
        success: true,
        message: "Token monitor started".to_string(),
    })
}

async fn stop_token_monitor(State(state): State<AppState>) -> Json<ApiResponse> {
    state.token_monitor.stop();
    Json(ApiResponse {
        success: true,
        message: "Token monitor stopped".to_string(),
    })
}

async fn set_token_filter(
    State(state): State<AppState>,
    Json(req): Json<FilterRequest>,
) -> Json<ApiResponse> {
    state.token_monitor.set_filter(&req.key, req.value);
    Json(ApiResponse {
        success: true,
        message: format!("Filter {} set to {}", req.key, req.value),
    })
}

async fn blacklist_creator(
    State(state): State<AppState>,
    Json(req): Json<AddressRequest>,
) -> Json<ApiResponse> {
    state.token_monitor.blacklist_creator(&req.address);
    Json(ApiResponse {
        success: true,
        message: format!("Creator {} blacklisted", req.address),
    })
}

// Rug Detector handlers
async fn get_watched_tokens(
    State(state): State<AppState>,
) -> Json<Vec<crate::modules::rug_detector::WatchedToken>> {
    Json(state.rug_detector.get_watched_tokens())
}

async fn get_token_details(
    State(state): State<AppState>,
    Path(mint): Path<String>,
) -> Response {
    match state.rug_detector.get_token_details(&mint) {
        Some(token) => Json(token).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Token not found"}))).into_response(),
    }
}

async fn watch_token(
    State(state): State<AppState>,
    Json(req): Json<WatchTokenRequest>,
) -> Json<ApiResponse> {
    state.rug_detector.watch_token(
        &req.mint,
        &req.name,
        &req.symbol,
        &req.creator,
        req.initial_liquidity,
    );
    Json(ApiResponse {
        success: true,
        message: format!("Now watching token {}", req.symbol),
    })
}

async fn start_rug_detector(State(state): State<AppState>) -> Json<ApiResponse> {
    let _ = state.rug_detector.start().await;
    Json(ApiResponse {
        success: true,
        message: "Rug detector started".to_string(),
    })
}

async fn stop_rug_detector(State(state): State<AppState>) -> Json<ApiResponse> {
    state.rug_detector.stop();
    Json(ApiResponse {
        success: true,
        message: "Rug detector stopped".to_string(),
    })
}

// Whale Watcher handlers
async fn get_whales(
    State(state): State<AppState>,
) -> Json<Vec<crate::modules::whale_watcher::WatchedWallet>> {
    Json(state.whale_watcher.get_whales())
}

async fn get_top_movers(
    State(state): State<AppState>,
) -> Json<Vec<crate::modules::whale_watcher::TopMover>> {
    Json(state.whale_watcher.get_top_movers(10))
}

async fn get_wallet_activity(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Response {
    match state.whale_watcher.get_wallet_activity(&address) {
        Some(wallet) => Json(wallet).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Wallet not found"}))).into_response(),
    }
}

async fn watch_wallet(
    State(state): State<AppState>,
    Json(req): Json<WatchWalletRequest>,
) -> Json<ApiResponse> {
    state.whale_watcher.watch_wallet(&req.address, req.label.as_deref().unwrap_or(""));
    Json(ApiResponse {
        success: true,
        message: format!("Now watching wallet {}", req.address),
    })
}

async fn start_whale_watcher(State(state): State<AppState>) -> Json<ApiResponse> {
    let _ = state.whale_watcher.start().await;
    Json(ApiResponse {
        success: true,
        message: "Whale watcher started".to_string(),
    })
}

async fn stop_whale_watcher(State(state): State<AppState>) -> Json<ApiResponse> {
    state.whale_watcher.stop();
    Json(ApiResponse {
        success: true,
        message: "Whale watcher stopped".to_string(),
    })
}

// Alerts handler
async fn get_alerts(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Json<Vec<Alert>> {
    let limit = params.limit.unwrap_or(50);
    Json(state.alerts.get_recent_alerts(limit))
}

// Database handlers
async fn get_db_tokens(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Json<Vec<crate::utils::database::TokenRecord>> {
    let limit = params.limit.unwrap_or(50) as i64;
    match state.database.get_recent_tokens(limit) {
        Ok(tokens) => Json(tokens),
        Err(_) => Json(vec![]),
    }
}

async fn get_db_token(
    State(state): State<AppState>,
    Path(mint): Path<String>,
) -> Response {
    match state.database.get_token(&mint) {
        Ok(Some(token)) => Json(token).into_response(),
        _ => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Token not found"}))).into_response(),
    }
}

// Metrics handler
async fn get_metrics(State(state): State<AppState>) -> Response {
    // Update current state metrics
    state.metrics.pending_tokens.set(state.token_monitor.detected_tokens().len() as f64);
    state.metrics.tokens_watched.set(state.rug_detector.watched_tokens.len() as f64);
    state.metrics.set_module_status("tokenMonitor", state.token_monitor.is_running());
    state.metrics.set_module_status("rugDetector", state.rug_detector.is_running());
    state.metrics.set_module_status("whaleWatcher", state.whale_watcher.is_running());

    let metrics = state.metrics.get_metrics();
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        metrics,
    )
        .into_response()
}

// Health check handlers
async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        uptime: state.start_time.elapsed().as_secs_f64(),
        modules: ModuleStatus {
            token_monitor: state.token_monitor.is_running(),
            rug_detector: state.rug_detector.is_running(),
            whale_watcher: state.whale_watcher.is_running(),
        },
    })
}

async fn readiness_check(State(state): State<AppState>) -> Response {
    let ready = state.token_monitor.is_running()
        || state.rug_detector.is_running()
        || state.whale_watcher.is_running();

    if ready {
        Json(serde_json::json!({"ready": true})).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"ready": false}))).into_response()
    }
}

// WebSocket handler
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}

async fn handle_websocket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    info!(target: "DASHBOARD", "WebSocket client connected");

    // Send initial state
    let init_msg = WsMessage::Init {
        stats: StatsResponse {
            token_monitor: state.token_monitor.get_stats(),
            rug_detector: state.rug_detector.get_stats(),
            whale_watcher: state.whale_watcher.get_stats(),
        },
        recent_alerts: state.alerts.get_recent_alerts(20),
    };

    if let Ok(json) = serde_json::to_string(&init_msg) {
        let _ = sender.send(Message::Text(json)).await;
    }

    // Subscribe to alerts
    let mut alert_rx = state.alerts.subscribe();

    // Forward alerts to websocket
    let send_task = tokio::spawn(async move {
        while let Ok(alert) = alert_rx.recv().await {
            let msg = WsMessage::Alert(alert);
            if let Ok(json) = serde_json::to_string(&msg) {
                if sender.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    // Handle incoming messages (mainly for keeping connection alive)
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Close(_)) => break,
                Ok(Message::Ping(_)) => {
                    // Pong is handled automatically by axum
                }
                Err(_) => break,
                _ => {}
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    info!(target: "DASHBOARD", "WebSocket client disconnected");
}

