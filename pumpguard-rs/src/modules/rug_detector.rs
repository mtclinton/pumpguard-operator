//! Rug Pull Detector - Monitors for suspicious activity and rug pulls

use anyhow::Result;
use chrono::Utc;
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiMessage,
    option_serializer::OptionSerializer,
};
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::utils::alerts::TokenAlertInfo;
use crate::utils::database::TransactionRecord;
use crate::utils::{AlertService, DatabaseService, SolanaService};

/// Sell transaction info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SellInfo {
    pub signature: String,
    pub wallet: String,
    pub amount_sol: f64,
    pub amount_tokens: f64,
    pub timestamp: i64,
}

/// Alert info for rug detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RugAlert {
    pub alert_type: String,
    pub message: String,
    pub severity: String,
}

/// Watched token with rug detection data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedToken {
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub creator: String,
    pub initial_liquidity: f64,
    pub current_liquidity: f64,
    pub dev_wallet: String,
    pub sell_history: VecDeque<SellInfo>,
    pub last_check: i64,
    pub suspicion_score: i32,
    pub alerts: Vec<RugAlert>,
    pub is_rugged: bool,
    pub rug_reason: Option<String>,
}

/// Rug detection thresholds
#[derive(Debug, Clone)]
pub struct RugThresholds {
    pub lp_removal_percent: f64,
    pub suspicious_sell_percent: f64,
    pub dev_wallet_sell_alert: bool,
    pub max_dev_sell_percent: f64,
    pub min_time_between_sells: i64,
    pub holder_concentration_alert: f64,
}

/// Rug detector statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RugDetectorStats {
    pub tokens_watched: u64,
    pub rugs_detected: u64,
    pub alerts_sent: u64,
    pub watched_tokens: usize,
    pub is_running: bool,
}

/// Parsed sell info from transaction
struct ParsedSellInfo {
    mint: String,
    wallet: String,
    amount_sol: f64,
    amount_tokens: f64,
}

/// Rug Pull Detector module
pub struct RugDetector {
    config: Config,
    solana: Arc<SolanaService>,
    alerts: Arc<AlertService>,
    database: Arc<DatabaseService>,

    is_running: Arc<AtomicBool>,
    pub watched_tokens: Arc<DashMap<String, WatchedToken>>,
    thresholds: Arc<RwLock<RugThresholds>>,

    tokens_watched: Arc<AtomicU64>,
    rugs_detected: Arc<AtomicU64>,
    alerts_sent: Arc<AtomicU64>,
}

impl RugDetector {
    /// Create a new rug detector
    pub fn new(
        config: Config,
        solana: Arc<SolanaService>,
        alerts: Arc<AlertService>,
        database: Arc<DatabaseService>,
    ) -> Self {
        let thresholds = RugThresholds {
            lp_removal_percent: config.lp_removal_threshold_percent,
            suspicious_sell_percent: config.suspicious_sell_percent,
            dev_wallet_sell_alert: config.dev_wallet_sell_alert,
            max_dev_sell_percent: 20.0,
            min_time_between_sells: 60000, // 1 minute
            holder_concentration_alert: 80.0,
        };

        Self {
            config,
            solana,
            alerts,
            database,
            is_running: Arc::new(AtomicBool::new(false)),
            watched_tokens: Arc::new(DashMap::new()),
            thresholds: Arc::new(RwLock::new(thresholds)),
            tokens_watched: Arc::new(AtomicU64::new(0)),
            rugs_detected: Arc::new(AtomicU64::new(0)),
            alerts_sent: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Watch a token for rug detection
    pub fn watch_token(
        &self,
        mint: &str,
        name: &str,
        symbol: &str,
        creator: &str,
        initial_liquidity: f64,
    ) {
        if self.watched_tokens.contains_key(mint) {
            return;
        }

        let token = WatchedToken {
            mint: mint.to_string(),
            name: name.to_string(),
            symbol: symbol.to_string(),
            creator: creator.to_string(),
            initial_liquidity,
            current_liquidity: initial_liquidity,
            dev_wallet: creator.to_string(),
            sell_history: VecDeque::new(),
            last_check: Utc::now().timestamp_millis(),
            suspicion_score: 0,
            alerts: Vec::new(),
            is_rugged: false,
            rug_reason: None,
        };

        self.watched_tokens.insert(mint.to_string(), token);
        self.tokens_watched.fetch_add(1, Ordering::SeqCst);

        info!(
            target: "RUG_DETECTOR",
            "Now watching: {} ({})",
            symbol,
            SolanaService::shorten_address(mint, 4)
        );
    }

    /// Unwatch a token
    pub fn unwatch_token(&self, mint: &str) {
        self.watched_tokens.remove(mint);
        info!(
            target: "RUG_DETECTOR",
            "Stopped watching: {}",
            SolanaService::shorten_address(mint, 4)
        );
    }

    /// Start the rug detector
    pub async fn start(&self) -> Result<()> {
        if self.is_running.load(Ordering::SeqCst) {
            warn!(target: "RUG_DETECTOR", "Already running");
            return Ok(());
        }

        self.is_running.store(true, Ordering::SeqCst);
        info!(target: "RUG_DETECTOR", "🔍 Starting Rug Pull Detector...");

        // Subscribe to Solana logs for sell events
        let mut log_receiver = self.solana.subscribe_logs();

        let is_running = Arc::clone(&self.is_running);
        let solana = Arc::clone(&self.solana);
        let alerts = Arc::clone(&self.alerts);
        let database = Arc::clone(&self.database);
        let watched_tokens = Arc::clone(&self.watched_tokens);
        let thresholds = Arc::clone(&self.thresholds);
        let rugs_detected = Arc::clone(&self.rugs_detected);
        let alerts_sent = Arc::clone(&self.alerts_sent);

        // Log handler task
        tokio::spawn({
            let is_running = Arc::clone(&is_running);
            let watched_tokens = Arc::clone(&watched_tokens);
            let solana = Arc::clone(&solana);
            let alerts = Arc::clone(&alerts);
            let database = Arc::clone(&database);
            let thresholds = Arc::clone(&thresholds);
            let rugs_detected = Arc::clone(&rugs_detected);
            let alerts_sent = Arc::clone(&alerts_sent);

            async move {
                while is_running.load(Ordering::SeqCst) {
                    match log_receiver.recv().await {
                        Ok(log_event) => {
                            // Check for sell events
                            let is_sell = log_event
                                .logs
                                .iter()
                                .any(|log| log.contains("Program log: Instruction: Sell"));

                            if is_sell {
                                // Throttle processing
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                
                                if let Err(e) = Self::analyze_sell_transaction(
                                    &solana,
                                    &alerts,
                                    &database,
                                    &watched_tokens,
                                    &thresholds,
                                    &rugs_detected,
                                    &alerts_sent,
                                    &log_event.signature,
                                )
                                .await
                                {
                                    error!(target: "RUG_DETECTOR", "Error analyzing sell: {}", e);
                                }
                            }

                            // Check for LP removal
                            let is_lp_removal = log_event.logs.iter().any(|log| {
                                log.contains("withdraw")
                                    || log.contains("remove_liquidity")
                                    || log.contains("migrate")
                            });

                            if is_lp_removal {
                                if let Err(e) = Self::analyze_lp_removal(
                                    &solana,
                                    &alerts,
                                    &database,
                                    &watched_tokens,
                                    &thresholds,
                                    &rugs_detected,
                                    &alerts_sent,
                                    &log_event.signature,
                                )
                                .await
                                {
                                    error!(target: "RUG_DETECTOR", "Error analyzing LP removal: {}", e);
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(target: "RUG_DETECTOR", "Lagged {} messages", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        });

        // Health check task
        tokio::spawn({
            let is_running = Arc::clone(&is_running);
            let watched_tokens = Arc::clone(&watched_tokens);
            let solana = Arc::clone(&solana);
            let alerts = Arc::clone(&alerts);
            let database = Arc::clone(&database);
            let thresholds = Arc::clone(&thresholds);
            let rugs_detected = Arc::clone(&rugs_detected);
            let alerts_sent = Arc::clone(&alerts_sent);

            async move {
                let mut interval = interval(Duration::from_secs(30));

                while is_running.load(Ordering::SeqCst) {
                    interval.tick().await;

                    for entry in watched_tokens.iter() {
                        let mint = entry.key().clone();
                        let mut token = entry.value().clone();

                        // Skip if recently checked
                        if Utc::now().timestamp_millis() - token.last_check < 25000 {
                            continue;
                        }

                        token.last_check = Utc::now().timestamp_millis();

                        // Check liquidity health
                        if let Err(e) = Self::check_liquidity_health(
                            &solana,
                            &alerts,
                            &database,
                            &thresholds,
                            &rugs_detected,
                            &alerts_sent,
                            &mut token,
                        )
                        .await
                        {
                            error!(target: "RUG_DETECTOR", "Health check failed for {}: {}", token.symbol, e);
                        }

                        watched_tokens.insert(mint, token);
                    }
                }
            }
        });

        info!(target: "RUG_DETECTOR", "Rug Pull Detector active - monitoring for suspicious activity");
        Ok(())
    }

    /// Stop the rug detector
    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
        info!(target: "RUG_DETECTOR", "Rug Pull Detector stopping...");
    }

    async fn analyze_sell_transaction(
        solana: &Arc<SolanaService>,
        alerts: &Arc<AlertService>,
        database: &Arc<DatabaseService>,
        watched_tokens: &Arc<DashMap<String, WatchedToken>>,
        thresholds: &Arc<RwLock<RugThresholds>>,
        rugs_detected: &Arc<AtomicU64>,
        alerts_sent: &Arc<AtomicU64>,
        signature: &str,
    ) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(300)).await;

        let tx = match solana.get_transaction(signature).await? {
            Some(tx) => tx,
            None => return Ok(()),
        };

        let sell_info = match Self::parse_sell_transaction(&tx) {
            Some(info) => info,
            None => return Ok(()),
        };

        // Check if this token is being watched
        let mut token = match watched_tokens.get(&sell_info.mint) {
            Some(entry) => entry.value().clone(),
            None => return Ok(()),
        };

        // Record the sell
        token.sell_history.push_back(SellInfo {
            signature: signature.to_string(),
            wallet: sell_info.wallet.clone(),
            amount_sol: sell_info.amount_sol,
            amount_tokens: sell_info.amount_tokens,
            timestamp: Utc::now().timestamp_millis(),
        });

        // Keep only last 100 sells
        while token.sell_history.len() > 100 {
            token.sell_history.pop_front();
        }

        // Save transaction
        let _ = database.save_transaction(&TransactionRecord {
            signature: signature.to_string(),
            mint: sell_info.mint.clone(),
            wallet: sell_info.wallet.clone(),
            tx_type: "sell".to_string(),
            amount_sol: sell_info.amount_sol,
            amount_tokens: sell_info.amount_tokens,
            timestamp: Utc::now().to_rfc3339(),
        });

        // Check for suspicious patterns
        Self::check_suspicious_patterns(
            alerts,
            database,
            thresholds,
            rugs_detected,
            alerts_sent,
            &mut token,
            &sell_info,
        )
        .await?;

        watched_tokens.insert(sell_info.mint.clone(), token);

        Ok(())
    }

    fn parse_sell_transaction(tx: &EncodedConfirmedTransactionWithStatusMeta) -> Option<ParsedSellInfo> {
        let meta = tx.transaction.meta.as_ref()?;

        // Get wallet from first signer
        let wallet = match &tx.transaction.transaction {
            EncodedTransaction::Json(ui_tx) => match &ui_tx.message {
                UiMessage::Parsed(msg) => msg.account_keys.first().map(|k| k.pubkey.clone()),
                UiMessage::Raw(msg) => msg.account_keys.first().cloned(),
            },
            _ => None,
        }?;

        // Get mint from token balances
        let mint = match &meta.pre_token_balances {
            OptionSerializer::Some(balances) => balances.first().map(|b| b.mint.clone()),
            _ => None,
        }?;

        // Calculate SOL amount
        let amount_sol = {
            let pre = &meta.pre_balances;
            let post = &meta.post_balances;
            if !pre.is_empty() && !post.is_empty() {
                let change = post[0] as i64 - pre[0] as i64;
                (change.abs() as f64) / 1_000_000_000.0
            } else {
                0.0
            }
        };

        // Calculate token amount (simplified)
        let amount_tokens = 0.0;

        Some(ParsedSellInfo {
            mint,
            wallet,
            amount_sol,
            amount_tokens,
        })
    }

    async fn check_suspicious_patterns(
        alerts: &Arc<AlertService>,
        database: &Arc<DatabaseService>,
        thresholds: &Arc<RwLock<RugThresholds>>,
        rugs_detected: &Arc<AtomicU64>,
        alerts_sent: &Arc<AtomicU64>,
        token: &mut WatchedToken,
        sell_info: &ParsedSellInfo,
    ) -> Result<()> {
        let thresholds = thresholds.read().clone();
        let mut rug_alerts = Vec::new();

        // 1. Dev wallet selling
        if sell_info.wallet == token.dev_wallet {
            let sell_percent = (sell_info.amount_tokens / 1_000_000_000.0) * 100.0;

            if sell_percent >= thresholds.max_dev_sell_percent {
                rug_alerts.push(RugAlert {
                    alert_type: "dev_dump".to_string(),
                    message: format!("Developer sold {:.2}% of supply", sell_percent),
                    severity: "critical".to_string(),
                });
                token.suspicion_score += 50;
            } else if thresholds.dev_wallet_sell_alert {
                rug_alerts.push(RugAlert {
                    alert_type: "dev_sell".to_string(),
                    message: format!("Developer sold {:.4} SOL worth", sell_info.amount_sol),
                    severity: "medium".to_string(),
                });
                token.suspicion_score += 20;
            }
        }

        // 2. Rapid selling pattern
        let now = Utc::now().timestamp_millis();
        let recent_sells: Vec<_> = token
            .sell_history
            .iter()
            .filter(|s| now - s.timestamp < thresholds.min_time_between_sells)
            .collect();

        if recent_sells.len() >= 3 {
            let total_sold_sol: f64 = recent_sells.iter().map(|s| s.amount_sol).sum();
            if total_sold_sol > token.initial_liquidity * 0.3 {
                rug_alerts.push(RugAlert {
                    alert_type: "rapid_selling".to_string(),
                    message: format!(
                        "Rapid selling detected: {:.2} SOL in {} txs",
                        total_sold_sol,
                        recent_sells.len()
                    ),
                    severity: "high".to_string(),
                });
                token.suspicion_score += 30;
            }
        }

        // 3. Large single sell
        if token.current_liquidity > 0.0
            && sell_info.amount_sol
                > token.current_liquidity * (thresholds.suspicious_sell_percent / 100.0)
        {
            let percent = (sell_info.amount_sol / token.current_liquidity) * 100.0;
            rug_alerts.push(RugAlert {
                alert_type: "large_sell".to_string(),
                message: format!(
                    "Large sell: {:.4} SOL ({:.1}% of liquidity)",
                    sell_info.amount_sol, percent
                ),
                severity: "medium".to_string(),
            });
            token.suspicion_score += 15;
        }

        // 4. Check if this triggers rug threshold
        if token.suspicion_score >= 80 {
            Self::trigger_rug_alert(
                alerts,
                database,
                rugs_detected,
                alerts_sent,
                token,
                "High suspicion score reached",
            )
            .await?;
        }

        // Send alerts
        for alert in &rug_alerts {
            token.alerts.push(alert.clone());
            alerts_sent.fetch_add(1, Ordering::SeqCst);

            let token_info = TokenAlertInfo {
                mint: token.mint.clone(),
                name: token.name.clone(),
                symbol: token.symbol.clone(),
                creator: token.creator.clone(),
                initial_liquidity: Some(token.initial_liquidity),
            };

            if alert.severity == "critical" {
                error!(target: "RUG_ALERT", "🚨 {}: {}", token.symbol, alert.message);
                let _ = alerts
                    .alert_rug_pull(&token_info, &alert.message, &alert.severity)
                    .await;
            } else {
                warn!(target: "RUG_DETECTOR", "{}: {}", token.symbol, alert.message);
                let _ = alerts.alert_suspicious(&token_info, &alert.message).await;
            }
        }

        Ok(())
    }

    async fn trigger_rug_alert(
        alerts: &Arc<AlertService>,
        database: &Arc<DatabaseService>,
        rugs_detected: &Arc<AtomicU64>,
        alerts_sent: &Arc<AtomicU64>,
        token: &mut WatchedToken,
        reason: &str,
    ) -> Result<()> {
        rugs_detected.fetch_add(1, Ordering::SeqCst);

        error!(target: "RUG_ALERT", "🚨 RUG DETECTED: {} - {}", token.symbol, reason);

        // Mark as rugged in database
        let _ = database.mark_as_rugged(&token.mint, reason);

        // Send critical alert
        alerts_sent.fetch_add(1, Ordering::SeqCst);
        let _ = alerts
            .alert_rug_pull(
                &TokenAlertInfo {
                    mint: token.mint.clone(),
                    name: token.name.clone(),
                    symbol: token.symbol.clone(),
                    creator: token.creator.clone(),
                    initial_liquidity: Some(token.initial_liquidity),
                },
                reason,
                "critical",
            )
            .await;

        // Update token status
        token.is_rugged = true;
        token.rug_reason = Some(reason.to_string());

        Ok(())
    }

    async fn analyze_lp_removal(
        solana: &Arc<SolanaService>,
        alerts: &Arc<AlertService>,
        database: &Arc<DatabaseService>,
        watched_tokens: &Arc<DashMap<String, WatchedToken>>,
        thresholds: &Arc<RwLock<RugThresholds>>,
        rugs_detected: &Arc<AtomicU64>,
        alerts_sent: &Arc<AtomicU64>,
        signature: &str,
    ) -> Result<()> {
        let tx = match solana.get_transaction(signature).await? {
            Some(tx) => tx,
            None => return Ok(()),
        };

        // Check if this affects any watched tokens
        if let Some(meta) = &tx.transaction.meta {
            if let OptionSerializer::Some(pre_balances) = &meta.pre_token_balances {
                for balance in pre_balances {
                    if let Some(entry) = watched_tokens.get(&balance.mint) {
                        let mut token = entry.value().clone();
                        let thresholds = thresholds.read().clone();

                        // Calculate liquidity change
                        let pre = &meta.pre_balances;
                        let post = &meta.post_balances;
                        if !pre.is_empty() && !post.is_empty() {
                            let lp_change =
                                (pre[0] as i64 - post[0] as i64) as f64 / 1_000_000_000.0;

                            if token.current_liquidity > 0.0
                                && lp_change
                                    > token.current_liquidity
                                        * (thresholds.lp_removal_percent / 100.0)
                            {
                                let reason = format!(
                                    "LP removed: {:.2} SOL ({:.1}%)",
                                    lp_change,
                                    (lp_change / token.current_liquidity) * 100.0
                                );
                                Self::trigger_rug_alert(
                                    alerts,
                                    database,
                                    rugs_detected,
                                    alerts_sent,
                                    &mut token,
                                    &reason,
                                )
                                .await?;

                                watched_tokens.insert(balance.mint.clone(), token);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn check_liquidity_health(
        solana: &Arc<SolanaService>,
        alerts: &Arc<AlertService>,
        database: &Arc<DatabaseService>,
        thresholds: &Arc<RwLock<RugThresholds>>,
        rugs_detected: &Arc<AtomicU64>,
        alerts_sent: &Arc<AtomicU64>,
        token: &mut WatchedToken,
    ) -> Result<()> {
        // Get bonding curve balance
        let mint_pubkey = Pubkey::from_str(&token.mint)?;
        let bonding_curve = solana.derive_bonding_curve(&mint_pubkey);
        let balance = solana.get_balance(&bonding_curve.to_string()).await?;

        let previous_liquidity = token.current_liquidity;
        token.current_liquidity = balance;

        // Check for significant drop
        if previous_liquidity > 0.0 {
            let drop_percent = ((previous_liquidity - balance) / previous_liquidity) * 100.0;
            let thresholds = thresholds.read().clone();

            if drop_percent >= thresholds.lp_removal_percent {
                let reason = format!("Liquidity dropped {:.1}%", drop_percent);
                Self::trigger_rug_alert(
                    alerts,
                    database,
                    rugs_detected,
                    alerts_sent,
                    token,
                    &reason,
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Get detector statistics
    pub fn get_stats(&self) -> RugDetectorStats {
        RugDetectorStats {
            tokens_watched: self.tokens_watched.load(Ordering::SeqCst),
            rugs_detected: self.rugs_detected.load(Ordering::SeqCst),
            alerts_sent: self.alerts_sent.load(Ordering::SeqCst),
            watched_tokens: self.watched_tokens.len(),
            is_running: self.is_running.load(Ordering::SeqCst),
        }
    }

    /// Get list of watched tokens
    pub fn get_watched_tokens(&self) -> Vec<WatchedToken> {
        self.watched_tokens
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }

    /// Get details for a specific token
    pub fn get_token_details(&self, mint: &str) -> Option<WatchedToken> {
        self.watched_tokens.get(mint).map(|e| e.value().clone())
    }

    /// Check if running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }
}

impl Clone for RugDetector {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            solana: Arc::clone(&self.solana),
            alerts: Arc::clone(&self.alerts),
            database: Arc::clone(&self.database),
            is_running: Arc::clone(&self.is_running),
            watched_tokens: Arc::clone(&self.watched_tokens),
            thresholds: Arc::clone(&self.thresholds),
            tokens_watched: Arc::clone(&self.tokens_watched),
            rugs_detected: Arc::clone(&self.rugs_detected),
            alerts_sent: Arc::clone(&self.alerts_sent),
        }
    }
}
