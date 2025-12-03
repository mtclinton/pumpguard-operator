//! Whale Watcher - Tracks large wallet movements

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
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::utils::alerts::TokenAlertInfo;
use crate::utils::database::{TransactionRecord, WalletRecord};
use crate::utils::{AlertService, DatabaseService, SolanaService};

/// Transaction info for whale tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxInfo {
    pub signature: String,
    pub wallet: String,
    pub mint: String,
    pub tx_type: String,
    pub amount_sol: f64,
    pub amount_tokens: f64,
    pub timestamp: i64,
}

/// Watched wallet data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedWallet {
    pub address: String,
    pub label: String,
    pub total_volume: f64,
    pub is_whale: bool,
    pub transactions: VecDeque<TxInfo>,
    pub last_activity: Option<String>,
}

/// Token movement tracking
#[derive(Debug, Clone)]
pub struct TokenMovement {
    pub mint: String,
    pub buys: VecDeque<TxInfo>,
    pub sells: VecDeque<TxInfo>,
    pub net_flow: f64,
    pub unique_buyers: HashSet<String>,
    pub unique_sellers: HashSet<String>,
}

/// Whale watcher thresholds
#[derive(Debug, Clone)]
pub struct WhaleThresholds {
    pub whale_threshold_sol: f64,
    pub alert_on_accumulation: bool,
    pub alert_on_dump: bool,
    pub accumulation_window_ms: i64,
    pub min_transactions_for_pattern: usize,
}

/// Whale watcher statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhaleWatcherStats {
    pub wallets_tracked: u64,
    pub whales_identified: u64,
    pub accumulation_alerts: u64,
    pub dump_alerts: u64,
    pub total_volume_tracked: f64,
    pub watched_wallets: usize,
    pub tokens_tracked: usize,
    pub is_running: bool,
}

/// Top mover info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopMover {
    pub mint: String,
    pub net_flow: f64,
    pub volume: f64,
}

/// Whale Watcher module
pub struct WhaleWatcher {
    config: Config,
    solana: Arc<SolanaService>,
    alerts: Arc<AlertService>,
    database: Arc<DatabaseService>,

    is_running: Arc<AtomicBool>,
    watched_wallets: Arc<DashMap<String, WatchedWallet>>,
    token_movements: Arc<DashMap<String, TokenMovement>>,
    thresholds: Arc<RwLock<WhaleThresholds>>,

    wallets_tracked: Arc<AtomicU64>,
    whales_identified: Arc<AtomicU64>,
    accumulation_alerts: Arc<AtomicU64>,
    dump_alerts: Arc<AtomicU64>,
    total_volume_tracked: Arc<RwLock<f64>>,
}

impl WhaleWatcher {
    /// Create a new whale watcher
    pub fn new(
        config: Config,
        solana: Arc<SolanaService>,
        alerts: Arc<AlertService>,
        database: Arc<DatabaseService>,
    ) -> Self {
        let thresholds = WhaleThresholds {
            whale_threshold_sol: config.whale_threshold_sol,
            alert_on_accumulation: config.alert_on_accumulation,
            alert_on_dump: config.alert_on_dump,
            accumulation_window_ms: 3600000, // 1 hour
            min_transactions_for_pattern: 3,
        };

        Self {
            config,
            solana,
            alerts,
            database,
            is_running: Arc::new(AtomicBool::new(false)),
            watched_wallets: Arc::new(DashMap::new()),
            token_movements: Arc::new(DashMap::new()),
            thresholds: Arc::new(RwLock::new(thresholds)),
            wallets_tracked: Arc::new(AtomicU64::new(0)),
            whales_identified: Arc::new(AtomicU64::new(0)),
            accumulation_alerts: Arc::new(AtomicU64::new(0)),
            dump_alerts: Arc::new(AtomicU64::new(0)),
            total_volume_tracked: Arc::new(RwLock::new(0.0)),
        }
    }

    /// Watch a wallet
    pub fn watch_wallet(&self, address: &str, label: &str) {
        if self.watched_wallets.contains_key(address) {
            info!(
                target: "WHALE_WATCHER",
                "Already watching: {}",
                SolanaService::shorten_address(address, 4)
            );
            return;
        }

        let wallet = WatchedWallet {
            address: address.to_string(),
            label: label.to_string(),
            total_volume: 0.0,
            is_whale: false,
            transactions: VecDeque::new(),
            last_activity: None,
        };

        self.watched_wallets.insert(address.to_string(), wallet);

        let _ = self.database.save_wallet(&WalletRecord {
            address: address.to_string(),
            label: label.to_string(),
            total_volume_sol: 0.0,
            last_activity: None,
            is_whale: false,
        });

        self.wallets_tracked.fetch_add(1, Ordering::SeqCst);

        info!(
            target: "WHALE_WATCHER",
            "🐋 Now watching wallet: {}",
            if label.is_empty() {
                SolanaService::shorten_address(address, 4)
            } else {
                label.to_string()
            }
        );
    }

    /// Unwatch a wallet
    pub fn unwatch_wallet(&self, address: &str) {
        self.watched_wallets.remove(address);
        info!(
            target: "WHALE_WATCHER",
            "Stopped watching: {}",
            SolanaService::shorten_address(address, 4)
        );
    }

    /// Load known whales from database
    async fn load_known_whales(&self) -> Result<()> {
        let whales = self.database.get_whales()?;
        for whale in whales {
            self.watched_wallets.insert(
                whale.address.clone(),
                WatchedWallet {
                    address: whale.address,
                    label: whale.label,
                    total_volume: whale.total_volume_sol,
                    is_whale: true,
                    transactions: VecDeque::new(),
                    last_activity: whale.last_activity,
                },
            );
        }
        info!(
            target: "WHALE_WATCHER",
            "Loaded {} known whales",
            self.watched_wallets.len()
        );
        Ok(())
    }

    /// Start the whale watcher
    pub async fn start(&self) -> Result<()> {
        if self.is_running.load(Ordering::SeqCst) {
            warn!(target: "WHALE_WATCHER", "Already running");
            return Ok(());
        }

        self.is_running.store(true, Ordering::SeqCst);
        info!(target: "WHALE_WATCHER", "🐋 Starting Whale Watcher...");

        // Load known whales from database
        self.load_known_whales().await?;

        // Subscribe to Solana logs
        let mut log_receiver = self.solana.subscribe_logs();

        let is_running = Arc::clone(&self.is_running);
        let solana = Arc::clone(&self.solana);
        let alerts = Arc::clone(&self.alerts);
        let database = Arc::clone(&self.database);
        let watched_wallets = Arc::clone(&self.watched_wallets);
        let token_movements = Arc::clone(&self.token_movements);
        let thresholds = Arc::clone(&self.thresholds);
        let whales_identified = Arc::clone(&self.whales_identified);
        let accumulation_alerts = Arc::clone(&self.accumulation_alerts);
        let dump_alerts = Arc::clone(&self.dump_alerts);
        let total_volume_tracked = Arc::clone(&self.total_volume_tracked);

        // Log handler task
        tokio::spawn({
            let is_running = Arc::clone(&is_running);
            async move {
                while is_running.load(Ordering::SeqCst) {
                    match log_receiver.recv().await {
                        Ok(log_event) => {
                            // Check for buy/sell events
                            let is_buy = log_event
                                .logs
                                .iter()
                                .any(|log| log.contains("Program log: Instruction: Buy"));
                            let is_sell = log_event
                                .logs
                                .iter()
                                .any(|log| log.contains("Program log: Instruction: Sell"));

                            if is_buy || is_sell {
                                let tx_type = if is_buy { "buy" } else { "sell" };
                                if let Err(e) = Self::analyze_transaction(
                                    &solana,
                                    &alerts,
                                    &database,
                                    &watched_wallets,
                                    &token_movements,
                                    &thresholds,
                                    &whales_identified,
                                    &accumulation_alerts,
                                    &dump_alerts,
                                    &total_volume_tracked,
                                    &log_event.signature,
                                    tx_type,
                                )
                                .await
                                {
                                    error!(target: "WHALE_WATCHER", "Error analyzing transaction: {}", e);
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(target: "WHALE_WATCHER", "Lagged {} messages", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        });

        // Pattern analysis task
        tokio::spawn({
            let is_running = Arc::clone(&is_running);
            let token_movements = Arc::clone(&self.token_movements);
            let thresholds = Arc::clone(&self.thresholds);
            let database = Arc::clone(&self.database);

            async move {
                let mut interval = interval(Duration::from_secs(60));

                while is_running.load(Ordering::SeqCst) {
                    interval.tick().await;
                    Self::analyze_patterns(&token_movements, &thresholds, &database).await;
                }
            }
        });

        info!(target: "WHALE_WATCHER", "🐋 Whale Watcher active - tracking large wallet movements");
        Ok(())
    }

    /// Stop the whale watcher
    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
        info!(target: "WHALE_WATCHER", "🐋 Whale Watcher stopping...");
    }

    async fn analyze_transaction(
        solana: &Arc<SolanaService>,
        alerts: &Arc<AlertService>,
        database: &Arc<DatabaseService>,
        watched_wallets: &Arc<DashMap<String, WatchedWallet>>,
        token_movements: &Arc<DashMap<String, TokenMovement>>,
        thresholds: &Arc<RwLock<WhaleThresholds>>,
        whales_identified: &Arc<AtomicU64>,
        accumulation_alerts: &Arc<AtomicU64>,
        dump_alerts: &Arc<AtomicU64>,
        total_volume_tracked: &Arc<RwLock<f64>>,
        signature: &str,
        tx_type: &str,
    ) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(300)).await;

        let tx = match solana.get_transaction(signature).await? {
            Some(tx) => tx,
            None => return Ok(()),
        };

        let tx_info = match Self::parse_transaction(&tx, tx_type) {
            Some(info) => info,
            None => return Ok(()),
        };

        let thresholds_val = thresholds.read().clone();

        // Check if this is a whale transaction
        if tx_info.amount_sol >= thresholds_val.whale_threshold_sol {
            Self::handle_whale_transaction(
                alerts,
                database,
                watched_wallets,
                thresholds,
                whales_identified,
                accumulation_alerts,
                dump_alerts,
                total_volume_tracked,
                &tx_info,
            )
            .await?;
        }

        // Track wallet activity
        Self::track_wallet_activity(watched_wallets, &thresholds_val, whales_identified, &tx_info);

        // Track token movement
        Self::track_token_movement(token_movements, &thresholds_val, &tx_info);

        Ok(())
    }

    fn parse_transaction(tx: &EncodedConfirmedTransactionWithStatusMeta, tx_type: &str) -> Option<TxInfo> {
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
        let balances = if tx_type == "buy" {
            match &meta.post_token_balances {
                OptionSerializer::Some(b) => Some(b),
                _ => None,
            }
        } else {
            match &meta.pre_token_balances {
                OptionSerializer::Some(b) => Some(b),
                _ => None,
            }
        };
        let mint = balances.and_then(|b| b.first()).map(|b| b.mint.clone())?;

        // Calculate SOL amount
        let amount_sol = {
            let pre = &meta.pre_balances;
            let post = &meta.post_balances;
            if !pre.is_empty() && !post.is_empty() {
                let change = (post[0] as i64 - pre[0] as i64).abs();
                change as f64 / 1_000_000_000.0
            } else {
                0.0
            }
        };

        let signature = match &tx.transaction.transaction {
            EncodedTransaction::Json(ui_tx) => ui_tx.signatures.first().cloned().unwrap_or_default(),
            _ => String::new(),
        };

        Some(TxInfo {
            signature,
            wallet,
            mint,
            tx_type: tx_type.to_string(),
            amount_sol,
            amount_tokens: 0.0, // Would need more parsing
            timestamp: Utc::now().timestamp_millis(),
        })
    }

    async fn handle_whale_transaction(
        alerts: &Arc<AlertService>,
        database: &Arc<DatabaseService>,
        watched_wallets: &Arc<DashMap<String, WatchedWallet>>,
        thresholds: &Arc<RwLock<WhaleThresholds>>,
        whales_identified: &Arc<AtomicU64>,
        accumulation_alerts: &Arc<AtomicU64>,
        dump_alerts: &Arc<AtomicU64>,
        total_volume_tracked: &Arc<RwLock<f64>>,
        tx_info: &TxInfo,
    ) -> Result<()> {
        let thresholds_val = thresholds.read().clone();

        // Get or create wallet entry
        let mut wallet_data = watched_wallets
            .get(&tx_info.wallet)
            .map(|e| e.value().clone())
            .unwrap_or_else(|| {
                whales_identified.fetch_add(1, Ordering::SeqCst);
                WatchedWallet {
                    address: tx_info.wallet.clone(),
                    label: String::new(),
                    total_volume: 0.0,
                    is_whale: true,
                    transactions: VecDeque::new(),
                    last_activity: None,
                }
            });

        // Mark as whale
        if !wallet_data.is_whale {
            wallet_data.is_whale = true;
            whales_identified.fetch_add(1, Ordering::SeqCst);
            info!(
                target: "WHALE_WATCHER",
                "🐋 New whale identified: {}",
                SolanaService::shorten_address(&tx_info.wallet, 4)
            );
        }

        // Update wallet data
        wallet_data.total_volume += tx_info.amount_sol;
        wallet_data.last_activity = Some(Utc::now().to_rfc3339());
        wallet_data.transactions.push_back(tx_info.clone());

        // Keep only recent transactions
        while wallet_data.transactions.len() > 100 {
            wallet_data.transactions.pop_front();
        }

        // Save to database
        let _ = database.save_wallet(&WalletRecord {
            address: tx_info.wallet.clone(),
            label: wallet_data.label.clone(),
            total_volume_sol: wallet_data.total_volume,
            last_activity: wallet_data.last_activity.clone(),
            is_whale: true,
        });

        let _ = database.save_transaction(&TransactionRecord {
            signature: tx_info.signature.clone(),
            mint: tx_info.mint.clone(),
            wallet: tx_info.wallet.clone(),
            tx_type: tx_info.tx_type.clone(),
            amount_sol: tx_info.amount_sol,
            amount_tokens: tx_info.amount_tokens,
            timestamp: Utc::now().to_rfc3339(),
        });

        // Get token info
        let token_info = database
            .get_token(&tx_info.mint)
            .ok()
            .flatten()
            .map(|t| TokenAlertInfo {
                mint: t.mint,
                name: t.name,
                symbol: t.symbol,
                creator: t.creator,
                initial_liquidity: Some(t.initial_liquidity),
            })
            .unwrap_or_else(|| TokenAlertInfo {
                mint: tx_info.mint.clone(),
                name: "UNKNOWN".to_string(),
                symbol: "UNK".to_string(),
                creator: String::new(),
                initial_liquidity: None,
            });

        // Log and alert
        if tx_info.tx_type == "buy" {
            info!(
                target: "WHALE_WATCHER",
                "🐋 Whale BUYING: {:.2} SOL of {} - wallet: {}",
                tx_info.amount_sol,
                token_info.symbol,
                SolanaService::shorten_address(&tx_info.wallet, 4)
            );

            if thresholds_val.alert_on_accumulation {
                accumulation_alerts.fetch_add(1, Ordering::SeqCst);
                let _ = alerts
                    .alert_whale(
                        "buy",
                        &tx_info.wallet,
                        &token_info,
                        tx_info.amount_sol,
                        tx_info.amount_tokens,
                    )
                    .await;
            }
        } else {
            info!(
                target: "WHALE_WATCHER",
                "🐋 Whale SELLING: {:.2} SOL of {} - wallet: {}",
                tx_info.amount_sol,
                token_info.symbol,
                SolanaService::shorten_address(&tx_info.wallet, 4)
            );

            if thresholds_val.alert_on_dump {
                dump_alerts.fetch_add(1, Ordering::SeqCst);
                let _ = alerts
                    .alert_whale(
                        "sell",
                        &tx_info.wallet,
                        &token_info,
                        tx_info.amount_sol,
                        tx_info.amount_tokens,
                    )
                    .await;
            }
        }

        {
            let mut volume = total_volume_tracked.write();
            *volume += tx_info.amount_sol;
        }

        watched_wallets.insert(tx_info.wallet.clone(), wallet_data);

        Ok(())
    }

    fn track_wallet_activity(
        watched_wallets: &Arc<DashMap<String, WatchedWallet>>,
        thresholds: &WhaleThresholds,
        whales_identified: &Arc<AtomicU64>,
        tx_info: &TxInfo,
    ) {
        let mut wallet_data = watched_wallets
            .get(&tx_info.wallet)
            .map(|e| e.value().clone())
            .unwrap_or_else(|| WatchedWallet {
                address: tx_info.wallet.clone(),
                label: String::new(),
                total_volume: 0.0,
                is_whale: false,
                transactions: VecDeque::new(),
                last_activity: None,
            });

        wallet_data.total_volume += tx_info.amount_sol;
        wallet_data.last_activity = Some(Utc::now().to_rfc3339());
        wallet_data.transactions.push_back(tx_info.clone());

        // Check if wallet has become a whale
        if !wallet_data.is_whale && wallet_data.total_volume >= thresholds.whale_threshold_sol * 2.0
        {
            wallet_data.is_whale = true;
            whales_identified.fetch_add(1, Ordering::SeqCst);
            info!(
                target: "WHALE_WATCHER",
                "🐋 Wallet promoted to whale status: {} ({:.2} SOL volume)",
                SolanaService::shorten_address(&tx_info.wallet, 4),
                wallet_data.total_volume
            );
        }

        watched_wallets.insert(tx_info.wallet.clone(), wallet_data);
    }

    fn track_token_movement(
        token_movements: &Arc<DashMap<String, TokenMovement>>,
        thresholds: &WhaleThresholds,
        tx_info: &TxInfo,
    ) {
        let mut token_data = token_movements
            .get(&tx_info.mint)
            .map(|e| e.value().clone())
            .unwrap_or_else(|| TokenMovement {
                mint: tx_info.mint.clone(),
                buys: VecDeque::new(),
                sells: VecDeque::new(),
                net_flow: 0.0,
                unique_buyers: HashSet::new(),
                unique_sellers: HashSet::new(),
            });

        if tx_info.tx_type == "buy" {
            token_data.buys.push_back(tx_info.clone());
            token_data.net_flow += tx_info.amount_sol;
            token_data.unique_buyers.insert(tx_info.wallet.clone());
        } else {
            token_data.sells.push_back(tx_info.clone());
            token_data.net_flow -= tx_info.amount_sol;
            token_data.unique_sellers.insert(tx_info.wallet.clone());
        }

        // Keep only recent data
        let cutoff = Utc::now().timestamp_millis() - thresholds.accumulation_window_ms;
        token_data.buys.retain(|t| t.timestamp > cutoff);
        token_data.sells.retain(|t| t.timestamp > cutoff);

        token_movements.insert(tx_info.mint.clone(), token_data);
    }

    async fn analyze_patterns(
        token_movements: &Arc<DashMap<String, TokenMovement>>,
        thresholds: &Arc<RwLock<WhaleThresholds>>,
        database: &Arc<DatabaseService>,
    ) {
        let thresholds_val = thresholds.read().clone();

        for entry in token_movements.iter() {
            let data = entry.value();

            // Check for whale accumulation pattern
            let whale_buys: Vec<_> = data
                .buys
                .iter()
                .filter(|b| b.amount_sol >= thresholds_val.whale_threshold_sol)
                .collect();

            if whale_buys.len() >= thresholds_val.min_transactions_for_pattern {
                let total_accumulation: f64 = whale_buys.iter().map(|b| b.amount_sol).sum();
                let token_info = database
                    .get_token(&data.mint)
                    .ok()
                    .flatten()
                    .map(|t| t.symbol)
                    .unwrap_or_else(|| "UNKNOWN".to_string());

                info!(
                    target: "WHALE_WATCHER",
                    "🐋 Accumulation pattern detected for {}: {} whale buys totaling {:.2} SOL",
                    token_info,
                    whale_buys.len(),
                    total_accumulation
                );
            }

            // Check for coordinated selling
            let whale_sells: Vec<_> = data
                .sells
                .iter()
                .filter(|s| s.amount_sol >= thresholds_val.whale_threshold_sol)
                .collect();

            if whale_sells.len() >= thresholds_val.min_transactions_for_pattern {
                let total_dump: f64 = whale_sells.iter().map(|s| s.amount_sol).sum();
                let token_info = database
                    .get_token(&data.mint)
                    .ok()
                    .flatten()
                    .map(|t| t.symbol)
                    .unwrap_or_else(|| "UNKNOWN".to_string());

                warn!(
                    target: "WHALE_WATCHER",
                    "⚠️ Dump pattern detected for {}: {} whale sells totaling {:.2} SOL",
                    token_info,
                    whale_sells.len(),
                    total_dump
                );
            }
        }

        // Clean up old data
        token_movements.retain(|_, data| !data.buys.is_empty() || !data.sells.is_empty());
    }

    /// Get watcher statistics
    pub fn get_stats(&self) -> WhaleWatcherStats {
        WhaleWatcherStats {
            wallets_tracked: self.wallets_tracked.load(Ordering::SeqCst),
            whales_identified: self.whales_identified.load(Ordering::SeqCst),
            accumulation_alerts: self.accumulation_alerts.load(Ordering::SeqCst),
            dump_alerts: self.dump_alerts.load(Ordering::SeqCst),
            total_volume_tracked: *self.total_volume_tracked.read(),
            watched_wallets: self.watched_wallets.len(),
            tokens_tracked: self.token_movements.len(),
            is_running: self.is_running.load(Ordering::SeqCst),
        }
    }

    /// Get list of whales
    pub fn get_whales(&self) -> Vec<WatchedWallet> {
        self.watched_wallets
            .iter()
            .filter(|e| e.value().is_whale)
            .map(|e| {
                let mut w = e.value().clone();
                // Limit transactions in response
                while w.transactions.len() > 10 {
                    w.transactions.pop_front();
                }
                w
            })
            .collect()
    }

    /// Get wallet activity
    pub fn get_wallet_activity(&self, address: &str) -> Option<WatchedWallet> {
        self.watched_wallets.get(address).map(|e| e.value().clone())
    }

    /// Get top token movers
    pub fn get_top_movers(&self, limit: usize) -> Vec<TopMover> {
        let mut movers: Vec<_> = self
            .token_movements
            .iter()
            .map(|e| {
                let data = e.value();
                let buy_vol: f64 = data.buys.iter().map(|b| b.amount_sol).sum();
                let sell_vol: f64 = data.sells.iter().map(|s| s.amount_sol).sum();
                TopMover {
                    mint: data.mint.clone(),
                    net_flow: data.net_flow,
                    volume: buy_vol + sell_vol,
                }
            })
            .collect();

        movers.sort_by(|a, b| {
            b.net_flow
                .abs()
                .partial_cmp(&a.net_flow.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        movers.truncate(limit);
        movers
    }

    /// Check if running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }
}

impl Clone for WhaleWatcher {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            solana: Arc::clone(&self.solana),
            alerts: Arc::clone(&self.alerts),
            database: Arc::clone(&self.database),
            is_running: Arc::clone(&self.is_running),
            watched_wallets: Arc::clone(&self.watched_wallets),
            token_movements: Arc::clone(&self.token_movements),
            thresholds: Arc::clone(&self.thresholds),
            wallets_tracked: Arc::clone(&self.wallets_tracked),
            whales_identified: Arc::clone(&self.whales_identified),
            accumulation_alerts: Arc::clone(&self.accumulation_alerts),
            dump_alerts: Arc::clone(&self.dump_alerts),
            total_volume_tracked: Arc::clone(&self.total_volume_tracked),
        }
    }
}
