//! Token Monitor - Detects new token launches on pump.fun

use anyhow::Result;
use chrono::Utc;
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiMessage,
    option_serializer::OptionSerializer,
};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::utils::alerts::TokenAlertInfo;
use crate::utils::database::TokenRecord;
use crate::utils::{AlertService, DatabaseService, SolanaService};

/// Token information detected by the monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedToken {
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub creator: String,
    pub created_at: String,
    pub signature: String,
    pub initial_liquidity: f64,
    pub detected_at: i64,
}

/// Token monitor filters
#[derive(Debug, Clone)]
pub struct TokenFilters {
    pub min_liquidity_sol: f64,
    pub max_liquidity_sol: f64,
    pub blacklisted_creators: HashSet<String>,
    pub whitelisted_creators: HashSet<String>,
    pub max_alerts_per_minute: u32,
    pub alert_new_tokens: bool,
}

impl TokenFilters {
    pub fn from_config(config: &Config) -> Self {
        Self {
            min_liquidity_sol: config.min_liquidity_sol,
            max_liquidity_sol: f64::INFINITY,
            blacklisted_creators: HashSet::new(),
            whitelisted_creators: HashSet::new(),
            max_alerts_per_minute: config.max_alerts_per_minute,
            alert_new_tokens: config.alert_new_tokens,
        }
    }
}

impl Default for TokenFilters {
    fn default() -> Self {
        Self {
            min_liquidity_sol: 1.0, // Default: 1 SOL minimum
            max_liquidity_sol: f64::INFINITY,
            blacklisted_creators: HashSet::new(),
            whitelisted_creators: HashSet::new(),
            max_alerts_per_minute: 10, // Default: 10 alerts/min
            alert_new_tokens: true,
        }
    }
}

/// Token monitor statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenMonitorStats {
    pub tokens_detected: u64,
    pub alerts_sent: u64,
    pub alerts_skipped: u64,
    pub tokens_tracked: usize,
    pub is_running: bool,
}

/// Rate limiter for alerts
struct AlertRateLimiter {
    timestamps: VecDeque<i64>,
    max_per_minute: u32,
}

impl AlertRateLimiter {
    fn new(max_per_minute: u32) -> Self {
        Self {
            timestamps: VecDeque::with_capacity(100),
            max_per_minute,
        }
    }

    fn can_send(&mut self) -> bool {
        if self.max_per_minute == 0 {
            return true; // Unlimited
        }

        let now = chrono::Utc::now().timestamp();
        let one_minute_ago = now - 60;

        // Remove old timestamps
        while let Some(&ts) = self.timestamps.front() {
            if ts < one_minute_ago {
                self.timestamps.pop_front();
            } else {
                break;
            }
        }

        if self.timestamps.len() < self.max_per_minute as usize {
            self.timestamps.push_back(now);
            true
        } else {
            false
        }
    }
}

/// Token Monitor module
pub struct TokenMonitor {
    config: Config,
    solana: Arc<SolanaService>,
    alerts: Arc<AlertService>,
    database: Arc<DatabaseService>,

    is_running: Arc<AtomicBool>,
    detected_tokens: Arc<DashMap<String, DetectedToken>>,
    filters: Arc<RwLock<TokenFilters>>,
    rate_limiter: Arc<RwLock<AlertRateLimiter>>,

    tokens_detected: Arc<AtomicU64>,
    alerts_sent: Arc<AtomicU64>,
    alerts_skipped: Arc<AtomicU64>,

    new_token_sender: broadcast::Sender<DetectedToken>,
}

impl TokenMonitor {
    /// Create a new token monitor
    pub fn new(
        config: Config,
        solana: Arc<SolanaService>,
        alerts: Arc<AlertService>,
        database: Arc<DatabaseService>,
    ) -> Self {
        let (new_token_sender, _) = broadcast::channel(10000);
        let filters = TokenFilters::from_config(&config);
        let rate_limiter = AlertRateLimiter::new(config.max_alerts_per_minute);

        info!(
            target: "TOKEN_MONITOR",
            "Filters: min_liquidity={} SOL, max_alerts/min={}, alerts_enabled={}",
            filters.min_liquidity_sol,
            filters.max_alerts_per_minute,
            filters.alert_new_tokens
        );

        Self {
            config,
            solana,
            alerts,
            database,
            is_running: Arc::new(AtomicBool::new(false)),
            detected_tokens: Arc::new(DashMap::new()),
            filters: Arc::new(RwLock::new(filters)),
            rate_limiter: Arc::new(RwLock::new(rate_limiter)),
            tokens_detected: Arc::new(AtomicU64::new(0)),
            alerts_sent: Arc::new(AtomicU64::new(0)),
            alerts_skipped: Arc::new(AtomicU64::new(0)),
            new_token_sender,
        }
    }

    /// Subscribe to new token events
    pub fn subscribe_new_tokens(&self) -> broadcast::Receiver<DetectedToken> {
        self.new_token_sender.subscribe()
    }

    /// Start the token monitor
    pub async fn start(&self) -> Result<()> {
        if self.is_running.load(Ordering::SeqCst) {
            warn!(target: "TOKEN_MONITOR", "Already running");
            return Ok(());
        }

        self.is_running.store(true, Ordering::SeqCst);
        info!(target: "TOKEN_MONITOR", "🆕 Starting Token Monitor...");

        // Subscribe to Solana logs
        let mut log_receiver = self.solana.subscribe_logs();

        let is_running = Arc::clone(&self.is_running);
        let solana = Arc::clone(&self.solana);
        let alerts = Arc::clone(&self.alerts);
        let database = Arc::clone(&self.database);
        let detected_tokens = Arc::clone(&self.detected_tokens);
        let filters = Arc::clone(&self.filters);
        let rate_limiter = Arc::clone(&self.rate_limiter);
        let tokens_detected = Arc::clone(&self.tokens_detected);
        let alerts_sent = Arc::clone(&self.alerts_sent);
        let alerts_skipped = Arc::clone(&self.alerts_skipped);
        let new_token_sender = self.new_token_sender.clone();

        tokio::spawn(async move {
            info!(target: "TOKEN_MONITOR", "Token Monitor active - watching for new token launches");

            while is_running.load(Ordering::SeqCst) {
                match log_receiver.recv().await {
                    Ok(log_event) => {
                        // Check for token creation
                        let is_create = log_event.logs.iter().any(|log| {
                            log.contains("Program log: Instruction: Create")
                                || log.contains("Program log: Instruction: Initialize")
                        });

                        if is_create {
                            // Throttle: small delay between processing
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            
                            if let Err(e) = Self::handle_new_token(
                                &solana,
                                &alerts,
                                &database,
                                &detected_tokens,
                                &filters,
                                &rate_limiter,
                                &tokens_detected,
                                &alerts_sent,
                                &alerts_skipped,
                                &new_token_sender,
                                &log_event.signature,
                            )
                            .await
                            {
                                error!(target: "TOKEN_MONITOR", "Error handling new token: {}", e);
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(target: "TOKEN_MONITOR", "Lagged {} messages", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }

            info!(target: "TOKEN_MONITOR", "Token Monitor stopped");
        });

        Ok(())
    }

    /// Stop the token monitor
    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
        info!(target: "TOKEN_MONITOR", "Token Monitor stopping...");
    }

    async fn handle_new_token(
        solana: &Arc<SolanaService>,
        alerts: &Arc<AlertService>,
        database: &Arc<DatabaseService>,
        detected_tokens: &Arc<DashMap<String, DetectedToken>>,
        filters: &Arc<RwLock<TokenFilters>>,
        rate_limiter: &Arc<RwLock<AlertRateLimiter>>,
        tokens_detected: &Arc<AtomicU64>,
        alerts_sent: &Arc<AtomicU64>,
        alerts_skipped: &Arc<AtomicU64>,
        new_token_sender: &broadcast::Sender<DetectedToken>,
        signature: &str,
    ) -> Result<()> {
        // Small delay to ensure transaction is confirmed
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let tx = match solana.get_transaction(signature).await? {
            Some(tx) => tx,
            None => return Ok(()),
        };

        let token_info = match Self::parse_token_creation(&tx) {
            Some(info) => info,
            None => return Ok(()),
        };

        // Check if we already have this token (duplicate detection)
        if detected_tokens.contains_key(&token_info.mint) {
            return Ok(());
        }

        // Check filters and get alert settings
        let (should_alert, alert_enabled) = {
            let filters = filters.read();

            // Check blacklist
            if filters.blacklisted_creators.contains(&token_info.creator) {
                return Ok(());
            }

            // Check whitelist
            if !filters.whitelisted_creators.is_empty()
                && !filters.whitelisted_creators.contains(&token_info.creator)
            {
                return Ok(());
            }

            // Check liquidity bounds
            let meets_liquidity = token_info.initial_liquidity >= filters.min_liquidity_sol
                && token_info.initial_liquidity <= filters.max_liquidity_sol;

            (meets_liquidity, filters.alert_new_tokens)
        };

        tokens_detected.fetch_add(1, Ordering::SeqCst);

        // Save to database (always save, regardless of filters)
        let _ = database.save_token(&TokenRecord {
            mint: token_info.mint.clone(),
            name: token_info.name.clone(),
            symbol: token_info.symbol.clone(),
            creator: token_info.creator.clone(),
            created_at: token_info.created_at.clone(),
            initial_liquidity: token_info.initial_liquidity,
            current_liquidity: token_info.initial_liquidity,
            holder_count: 0,
            is_rugged: false,
            rug_reason: None,
            last_updated: Utc::now().to_rfc3339(),
        });

        // Store in memory
        detected_tokens.insert(token_info.mint.clone(), token_info.clone());

        // Keep only last 1000 tokens
        if detected_tokens.len() > 1000 {
            if let Some(oldest) = detected_tokens.iter().min_by_key(|e| e.detected_at) {
                detected_tokens.remove(oldest.key());
            }
        }

        // Broadcast new token event (for rug detector linking)
        let _ = new_token_sender.send(token_info.clone());

        // Only send alert if it passes filters and alerts are enabled
        if should_alert && alert_enabled {
            // Check rate limiter
            let can_send = {
                let mut limiter = rate_limiter.write();
                limiter.can_send()
            };

            if can_send {
                alerts_sent.fetch_add(1, Ordering::SeqCst);

                info!(
                    target: "TOKEN_MONITOR",
                    "🆕 New token: {} ({}) - Mint: {} - Creator: {} - Liquidity: {:.2} SOL",
                    token_info.name,
                    token_info.symbol,
                    SolanaService::shorten_address(&token_info.mint, 4),
                    SolanaService::shorten_address(&token_info.creator, 4),
                    token_info.initial_liquidity
                );

                let _ = alerts
                    .alert_new_token(&TokenAlertInfo {
                        mint: token_info.mint.clone(),
                        name: token_info.name.clone(),
                        symbol: token_info.symbol.clone(),
                        creator: token_info.creator.clone(),
                        initial_liquidity: Some(token_info.initial_liquidity),
                    })
                    .await;
            } else {
                alerts_skipped.fetch_add(1, Ordering::SeqCst);
            }
        }

        Ok(())
    }

    fn parse_token_creation(tx: &EncodedConfirmedTransactionWithStatusMeta) -> Option<DetectedToken> {
        let meta = tx.transaction.meta.as_ref()?;

        // Try to extract mint from post token balances
        let mint = match &meta.post_token_balances {
            OptionSerializer::Some(balances) => balances.first().map(|b| b.mint.clone()),
            _ => None,
        }?;

        // Get creator from first signer (fee payer)
        let creator = match &tx.transaction.transaction {
            EncodedTransaction::Json(ui_tx) => match &ui_tx.message {
                UiMessage::Parsed(msg) => msg
                    .account_keys
                    .first()
                    .map(|k| k.pubkey.clone()),
                UiMessage::Raw(msg) => msg.account_keys.first().cloned(),
            },
            EncodedTransaction::LegacyBinary(_) | EncodedTransaction::Binary(_, _) => {
                // Binary encoded transactions - skip
                None
            }
            _ => None,
        }?;

        // Parse name/symbol from logs
        let mut name = "Unknown".to_string();
        let mut symbol = "UNK".to_string();

        if let OptionSerializer::Some(logs) = &meta.log_messages {
            for log in logs {
                if let Some(n) = log.strip_prefix("Program log: name: ") {
                    name = n.trim().to_string();
                }
                if let Some(s) = log.strip_prefix("Program log: symbol: ") {
                    symbol = s.trim().to_string();
                }
            }
        }

        // Calculate initial liquidity from SOL changes
        let initial_liquidity = {
            let pre = &meta.pre_balances;
            let post = &meta.post_balances;
            if !pre.is_empty() && !post.is_empty() {
                let diff = pre[0] as i64 - post[0] as i64;
                (diff.abs() as f64) / 1_000_000_000.0
            } else {
                0.0
            }
        };

        let signature = match &tx.transaction.transaction {
            EncodedTransaction::Json(ui_tx) => ui_tx.signatures.first().cloned().unwrap_or_default(),
            _ => String::new(),
        };

        Some(DetectedToken {
            mint,
            name,
            symbol,
            creator,
            created_at: Utc::now().to_rfc3339(),
            signature,
            initial_liquidity,
            detected_at: Utc::now().timestamp_millis(),
        })
    }

    /// Set a filter value
    pub fn set_filter(&self, key: &str, value: f64) {
        let mut filters = self.filters.write();
        match key {
            "min_liquidity_sol" => filters.min_liquidity_sol = value,
            "max_liquidity_sol" => filters.max_liquidity_sol = value,
            _ => {}
        }
        info!(target: "TOKEN_MONITOR", "Filter updated: {} = {}", key, value);
    }

    /// Blacklist a creator address
    pub fn blacklist_creator(&self, address: &str) {
        let mut filters = self.filters.write();
        filters.blacklisted_creators.insert(address.to_string());
        info!(target: "TOKEN_MONITOR", "Creator blacklisted: {}", address);
    }

    /// Whitelist a creator address
    pub fn whitelist_creator(&self, address: &str) {
        let mut filters = self.filters.write();
        filters.whitelisted_creators.insert(address.to_string());
        info!(target: "TOKEN_MONITOR", "Creator whitelisted: {}", address);
    }

    /// Get monitor statistics
    pub fn get_stats(&self) -> TokenMonitorStats {
        TokenMonitorStats {
            tokens_detected: self.tokens_detected.load(Ordering::SeqCst),
            alerts_sent: self.alerts_sent.load(Ordering::SeqCst),
            alerts_skipped: self.alerts_skipped.load(Ordering::SeqCst),
            tokens_tracked: self.detected_tokens.len(),
            is_running: self.is_running.load(Ordering::SeqCst),
        }
    }

    /// Get recent tokens
    pub fn get_recent_tokens(&self, limit: usize) -> Vec<DetectedToken> {
        let mut tokens: Vec<_> = self
            .detected_tokens
            .iter()
            .map(|e| e.value().clone())
            .collect();
        tokens.sort_by(|a, b| b.detected_at.cmp(&a.detected_at));
        tokens.truncate(limit);
        tokens
    }

    /// Get a specific token by mint
    pub fn get_token(&self, mint: &str) -> Option<DetectedToken> {
        self.detected_tokens.get(mint).map(|e| e.value().clone())
    }

    /// Check if running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Get detected tokens map (for rug detector linking)
    pub fn detected_tokens(&self) -> &Arc<DashMap<String, DetectedToken>> {
        &self.detected_tokens
    }
}

impl Clone for TokenMonitor {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            solana: Arc::clone(&self.solana),
            alerts: Arc::clone(&self.alerts),
            database: Arc::clone(&self.database),
            is_running: Arc::clone(&self.is_running),
            detected_tokens: Arc::clone(&self.detected_tokens),
            filters: Arc::clone(&self.filters),
            rate_limiter: Arc::clone(&self.rate_limiter),
            tokens_detected: Arc::clone(&self.tokens_detected),
            alerts_sent: Arc::clone(&self.alerts_sent),
            alerts_skipped: Arc::clone(&self.alerts_skipped),
            new_token_sender: self.new_token_sender.clone(),
        }
    }
}
