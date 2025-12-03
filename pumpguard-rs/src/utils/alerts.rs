//! Alert service for Telegram and WebSocket notifications

use anyhow::Result;
use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info};

use crate::config::Config;

/// Alert data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: i64,
    #[serde(rename = "type")]
    pub alert_type: String,
    pub title: String,
    pub message: String,
    pub data: serde_json::Value,
    pub timestamp: String,
}

/// Token info for alerts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenAlertInfo {
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub creator: String,
    pub initial_liquidity: Option<f64>,
}

/// Alert service for sending notifications
pub struct AlertService {
    config: Config,
    telegram_client: Option<reqwest::Client>,
    alert_history: Arc<RwLock<VecDeque<Alert>>>,
    alert_sender: broadcast::Sender<Alert>,
    next_id: Arc<RwLock<i64>>,
}

impl AlertService {
    /// Create a new alert service
    pub fn new(config: Config) -> Self {
        let telegram_client = if config.telegram_bot_token.is_some() {
            Some(reqwest::Client::new())
        } else {
            None
        };

        if telegram_client.is_some() {
            info!(target: "ALERTS", "Telegram bot initialized");
        }

        let (alert_sender, _) = broadcast::channel(1000);

        Self {
            config,
            telegram_client,
            alert_history: Arc::new(RwLock::new(VecDeque::with_capacity(1000))),
            alert_sender,
            next_id: Arc::new(RwLock::new(1)),
        }
    }

    /// Subscribe to alerts
    pub fn subscribe(&self) -> broadcast::Receiver<Alert> {
        self.alert_sender.subscribe()
    }

    /// Send an alert
    pub async fn send_alert(
        &self,
        alert_type: &str,
        title: &str,
        message: &str,
        data: serde_json::Value,
    ) -> Result<Alert> {
        let id = {
            let mut next_id = self.next_id.write();
            let id = *next_id;
            *next_id += 1;
            id
        };

        let alert = Alert {
            id,
            alert_type: alert_type.to_string(),
            title: title.to_string(),
            message: message.to_string(),
            data,
            timestamp: Utc::now().to_rfc3339(),
        };

        // Add to history
        {
            let mut history = self.alert_history.write();
            history.push_front(alert.clone());
            if history.len() > 1000 {
                history.truncate(500);
            }
        }

        // Broadcast to subscribers
        let _ = self.alert_sender.send(alert.clone());

        // Send to Telegram
        if let (Some(client), Some(token), Some(chat_id)) = (
            &self.telegram_client,
            &self.config.telegram_bot_token,
            &self.config.telegram_chat_id,
        ) {
            let emoji = self.get_emoji(alert_type);
            let telegram_message = format!("{} *{}*\n\n{}", emoji, title, message);

            let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
            let params = serde_json::json!({
                "chat_id": chat_id,
                "text": telegram_message,
                "parse_mode": "Markdown",
                "disable_web_page_preview": true,
            });

            if let Err(e) = client.post(&url).json(&params).send().await {
                error!(target: "ALERTS", "Telegram send failed: {}", e);
            }
        }

        Ok(alert)
    }

    fn get_emoji(&self, alert_type: &str) -> &'static str {
        match alert_type {
            "rug" => "🚨",
            "whale_buy" => "🐋📈",
            "whale_sell" => "🐋📉",
            "new_token" => "🆕",
            "suspicious" => "⚠️",
            "success" => "✅",
            "error" => "❌",
            _ => "📢",
        }
    }

    /// Get recent alerts
    pub fn get_recent_alerts(&self, limit: usize) -> Vec<Alert> {
        let history = self.alert_history.read();
        history.iter().take(limit).cloned().collect()
    }

    // ============================================
    // SPECIFIC ALERT METHODS
    // ============================================

    pub async fn alert_new_token(&self, token: &TokenAlertInfo) -> Result<Alert> {
        let liquidity = token
            .initial_liquidity
            .map(|l| format!("{:.2} SOL", l))
            .unwrap_or_else(|| "Unknown".to_string());

        let message = format!(
            "Token: {} ({})\nMint: `{}`\nCreator: `{}`\nLiquidity: {}",
            token.name, token.symbol, token.mint, token.creator, liquidity
        );

        self.send_alert(
            "new_token",
            "New Token Detected",
            &message,
            serde_json::to_value(token)?,
        )
        .await
    }

    pub async fn alert_rug_pull(
        &self,
        token: &TokenAlertInfo,
        reason: &str,
        severity: &str,
    ) -> Result<Alert> {
        let message = format!(
            "Token: {}\nMint: `{}`\nReason: {}",
            token.symbol, token.mint, reason
        );

        self.send_alert(
            "rug",
            &format!("RUG PULL DETECTED - {}", severity.to_uppercase()),
            &message,
            serde_json::json!({
                "token": token,
                "reason": reason,
                "severity": severity,
            }),
        )
        .await
    }

    pub async fn alert_whale(
        &self,
        tx_type: &str,
        wallet: &str,
        token: &TokenAlertInfo,
        amount_sol: f64,
        amount_tokens: f64,
    ) -> Result<Alert> {
        let action = if tx_type == "buy" {
            "ACCUMULATING"
        } else {
            "DUMPING"
        };

        let message = format!(
            "Wallet: `{}`\nToken: {}\nAmount: {:.2} SOL ({} tokens)",
            wallet, token.symbol, amount_sol, amount_tokens as i64
        );

        self.send_alert(
            &format!("whale_{}", tx_type),
            &format!("Whale {}", action),
            &message,
            serde_json::json!({
                "wallet": wallet,
                "token": token,
                "amount_sol": amount_sol,
                "amount_tokens": amount_tokens,
                "type": tx_type,
            }),
        )
        .await
    }

    pub async fn alert_suspicious(&self, token: &TokenAlertInfo, reason: &str) -> Result<Alert> {
        let message = format!(
            "Token: {}\nMint: `{}`\nReason: {}",
            token.symbol, token.mint, reason
        );

        self.send_alert(
            "suspicious",
            "Suspicious Activity",
            &message,
            serde_json::json!({
                "token": token,
                "reason": reason,
            }),
        )
        .await
    }
}

impl Clone for AlertService {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            telegram_client: self.telegram_client.clone(),
            alert_history: Arc::clone(&self.alert_history),
            alert_sender: self.alert_sender.clone(),
            next_id: Arc::clone(&self.next_id),
        }
    }
}

