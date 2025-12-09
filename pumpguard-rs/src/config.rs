//! Configuration module for PumpGuard

use std::env;

/// Application configuration loaded from environment variables
#[derive(Debug, Clone)]
pub struct Config {
    // Solana RPC (read-only, no wallet needed)
    pub rpc_url: String,
    pub ws_url: String,

    // Pump.fun
    pub pump_program_id: String,

    // Telegram Alerts
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,

    // Whale Watcher
    pub whale_threshold_sol: f64,
    pub alert_on_accumulation: bool,
    pub alert_on_dump: bool,

    // Rug Detection
    pub lp_removal_threshold_percent: f64,
    pub suspicious_sell_percent: f64,
    pub dev_wallet_sell_alert: bool,

    // Dashboard
    pub dashboard_port: u16,
}

impl Config {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();

        Self {
            rpc_url: env::var("SOLANA_RPC_URL")
                .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string()),
            ws_url: env::var("SOLANA_WS_URL")
                .unwrap_or_else(|_| "wss://api.mainnet-beta.solana.com".to_string()),

            pump_program_id: env::var("PUMP_PROGRAM_ID")
                .unwrap_or_else(|_| "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P".to_string()),

            telegram_bot_token: env::var("TELEGRAM_BOT_TOKEN").ok(),
            telegram_chat_id: env::var("TELEGRAM_CHAT_ID").ok(),

            whale_threshold_sol: env::var("WHALE_THRESHOLD_SOL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50.0),
            alert_on_accumulation: env::var("ALERT_ON_ACCUMULATION")
                .map(|v| v != "false")
                .unwrap_or(true),
            alert_on_dump: env::var("ALERT_ON_DUMP")
                .map(|v| v != "false")
                .unwrap_or(true),

            lp_removal_threshold_percent: env::var("LP_REMOVAL_THRESHOLD_PERCENT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50.0),
            suspicious_sell_percent: env::var("SUSPICIOUS_SELL_PERCENT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10.0),
            dev_wallet_sell_alert: env::var("DEV_WALLET_SELL_ALERT")
                .map(|v| v != "false")
                .unwrap_or(true),

            dashboard_port: env::var("DASHBOARD_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_env()
    }
}


