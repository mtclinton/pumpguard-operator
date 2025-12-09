//! PumpGuard - pump.fun Monitoring Suite
//!
//! A comprehensive monitoring tool for pump.fun that detects:
//! - New token launches
//! - Rug pulls and suspicious activity
//! - Whale wallet movements
//!
//! This is a **monitoring-only** tool - no wallet or trading functionality.

mod config;
mod dashboard;
mod modules;
mod utils;

use anyhow::Result;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info, warn};

use config::Config;
use dashboard::DashboardServer;
use modules::{RugDetector, TokenMonitor, WhaleWatcher};
use utils::{init_logger, AlertService, DatabaseService, MetricsService, SolanaService};

const BANNER: &str = r#"
    ╔═══════════════════════════════════════════════════════════════╗
    ║                                                               ║
    ║   ██████╗ ██╗   ██╗███╗   ███╗██████╗  ██████╗ ██╗   ██╗ █████╗ ██████╗ ██████╗  ║
    ║   ██╔══██╗██║   ██║████╗ ████║██╔══██╗██╔════╝ ██║   ██║██╔══██╗██╔══██╗██╔══██╗ ║
    ║   ██████╔╝██║   ██║██╔████╔██║██████╔╝██║  ███╗██║   ██║███████║██████╔╝██║  ██║ ║
    ║   ██╔═══╝ ██║   ██║██║╚██╔╝██║██╔═══╝ ██║   ██║██║   ██║██╔══██║██╔══██╗██║  ██║ ║
    ║   ██║     ╚██████╔╝██║ ╚═╝ ██║██║     ╚██████╔╝╚██████╔╝██║  ██║██║  ██║██████╔╝ ║
    ║   ╚═╝      ╚═════╝ ╚═╝     ╚═╝╚═╝      ╚═════╝  ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝╚═════╝  ║
    ║                                                               ║
    ║   🛡️  pump.fun Monitoring Suite (Monitor-Only Mode)           ║
    ║   🆕 New Tokens | 🚨 Rug Detector | 🐋 Whale Watcher          ║
    ║                                                               ║
    ╚═══════════════════════════════════════════════════════════════╝
"#;

/// PumpGuard application
pub struct PumpGuard {
    config: Config,
    solana: Arc<SolanaService>,
    database: Arc<DatabaseService>,
    alerts: Arc<AlertService>,
    metrics: Arc<MetricsService>,
    token_monitor: TokenMonitor,
    rug_detector: RugDetector,
    whale_watcher: WhaleWatcher,
}

impl PumpGuard {
    /// Create a new PumpGuard instance
    pub fn new() -> Result<Self> {
        let config = Config::from_env();

        // Initialize services
        let solana = Arc::new(SolanaService::new(config.clone()));
        let database = Arc::new(DatabaseService::new("data/pumpguard.db")?);
        let alerts = Arc::new(AlertService::new(config.clone()));
        let metrics = Arc::new(MetricsService::new());

        // Initialize modules
        let token_monitor = TokenMonitor::new(
            config.clone(),
            Arc::clone(&solana),
            Arc::clone(&alerts),
            Arc::clone(&database),
        );

        let rug_detector = RugDetector::new(
            config.clone(),
            Arc::clone(&solana),
            Arc::clone(&alerts),
            Arc::clone(&database),
        );

        let whale_watcher = WhaleWatcher::new(
            config.clone(),
            Arc::clone(&solana),
            Arc::clone(&alerts),
            Arc::clone(&database),
        );

        Ok(Self {
            config,
            solana,
            database,
            alerts,
            metrics,
            token_monitor,
            rug_detector,
            whale_watcher,
        })
    }

    /// Start PumpGuard
    pub async fn start(&self) -> Result<()> {
        println!("{}", BANNER);

        info!(target: "PUMPGUARD", "Initializing PumpGuard Monitor...");

        // Link modules FIRST - subscribe to events before starting modules
        // This ensures we don't miss any tokens during startup
        self.link_modules();

        // Start Solana WebSocket subscription
        self.solana.start_log_subscription().await?;

        // Start all modules
        info!(target: "PUMPGUARD", "Starting monitoring modules...");

        let (tm_result, rd_result, ww_result) = tokio::join!(
            self.token_monitor.start(),
            self.rug_detector.start(),
            self.whale_watcher.start(),
        );

        tm_result?;
        rd_result?;
        ww_result?;

        info!(target: "PUMPGUARD", "✅ All modules started successfully!");
        info!(target: "PUMPGUARD", "Dashboard: http://localhost:{}", self.config.dashboard_port);

        // Start dashboard server
        let dashboard = DashboardServer::new(
            self.config.clone(),
            self.token_monitor.clone(),
            self.rug_detector.clone(),
            self.whale_watcher.clone(),
            Arc::clone(&self.alerts),
            Arc::clone(&self.database),
            Arc::clone(&self.metrics),
        );

        dashboard.start().await?;

        Ok(())
    }

    /// Link modules together
    fn link_modules(&self) {
        // Subscribe to new tokens and add them to rug detector watch list
        // IMPORTANT: This must be called BEFORE starting the token monitor
        let mut new_token_rx = self.token_monitor.subscribe_new_tokens();
        let rug_detector = self.rug_detector.clone();

        tokio::spawn(async move {
            info!(target: "PUMPGUARD", "Token->RugDetector link active, waiting for tokens...");
            
            loop {
                match new_token_rx.recv().await {
                    Ok(token) => {
                        if !rug_detector.watched_tokens.contains_key(&token.mint) {
                            rug_detector.watch_token(
                                &token.mint,
                                &token.name,
                                &token.symbol,
                                &token.creator,
                                token.initial_liquidity,
                            );
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(target: "PUMPGUARD", "Token link lagged {} messages - some tokens may not be watched", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!(target: "PUMPGUARD", "Token broadcast channel closed");
                        break;
                    }
                }
            }
        });

        info!(target: "PUMPGUARD", "Modules linked - new tokens will be auto-watched by rug detector");
    }

    /// Graceful shutdown
    pub async fn shutdown(&self) {
        info!(target: "PUMPGUARD", "Shutting down...");

        self.token_monitor.stop();
        self.rug_detector.stop();
        self.whale_watcher.stop();

        info!(target: "PUMPGUARD", "✅ Shutdown complete");
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    init_logger();

    // Create and start PumpGuard
    let pumpguard = match PumpGuard::new() {
        Ok(pg) => pg,
        Err(e) => {
            error!(target: "PUMPGUARD", "Failed to initialize: {}", e);
            return Err(e);
        }
    };

    // Setup shutdown signal handler
    let shutdown_signal = async {
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("Failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to install signal handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
    };

    // Run the application
    tokio::select! {
        result = pumpguard.start() => {
            if let Err(e) = result {
                error!(target: "PUMPGUARD", "Fatal error: {}", e);
            }
        }
        _ = shutdown_signal => {
            pumpguard.shutdown().await;
        }
    }

    Ok(())
}


