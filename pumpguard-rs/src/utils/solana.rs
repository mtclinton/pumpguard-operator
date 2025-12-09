//! Solana RPC service for PumpGuard (read-only, no wallet)

use anyhow::Result;
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::RpcTransactionConfig,
};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Signature,
};
use solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding};
use std::{str::FromStr, sync::Arc};
use tokio::sync::broadcast;
use tracing::{info, error, warn};

use crate::config::Config;

/// Log event from Solana WebSocket subscription
#[derive(Debug, Clone)]
pub struct LogEvent {
    pub signature: String,
    pub logs: Vec<String>,
}

/// Solana service for RPC interactions
pub struct SolanaService {
    pub client: Arc<RpcClient>,
    pub pump_program_id: Pubkey,
    config: Config,
    log_sender: broadcast::Sender<LogEvent>,
}

impl SolanaService {
    /// Create a new Solana service
    pub fn new(config: Config) -> Self {
        let client = Arc::new(RpcClient::new_with_commitment(
            config.rpc_url.clone(),
            CommitmentConfig::confirmed(),
        ));

        let pump_program_id = Pubkey::from_str(&config.pump_program_id)
            .expect("Invalid pump program ID");

        let (log_sender, _) = broadcast::channel(10000);

        info!(target: "SOLANA", "Connected to Solana RPC (monitor-only mode)");

        Self {
            client,
            pump_program_id,
            config,
            log_sender,
        }
    }

    /// Get a receiver for log events
    pub fn subscribe_logs(&self) -> broadcast::Receiver<LogEvent> {
        self.log_sender.subscribe()
    }

    /// Start the WebSocket log subscription
    pub async fn start_log_subscription(&self) -> Result<()> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message};
        use std::sync::atomic::{AtomicU64, Ordering};

        let ws_url = self.config.ws_url.clone();
        let program_id = self.pump_program_id.to_string();
        let sender = self.log_sender.clone();

        // Spawn WebSocket connection handler
        tokio::spawn(async move {
            let mut reconnect_delay = 5;
            let message_count = Arc::new(AtomicU64::new(0));

            loop {
                match connect_async(&ws_url).await {
                    Ok((ws_stream, _)) => {
                        info!(target: "SOLANA", "WebSocket connected to {}", ws_url);
                        reconnect_delay = 5; // Reset delay on successful connection

                        let (mut write, mut read) = ws_stream.split();

                        // Subscribe to program logs
                        let subscribe_msg = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "logsSubscribe",
                            "params": [
                                {"mentions": [program_id]},
                                {"commitment": "confirmed"}
                            ]
                        });

                        if let Err(e) = write.send(Message::Text(subscribe_msg.to_string())).await {
                            error!(target: "SOLANA", "Failed to send subscribe message: {}", e);
                            continue;
                        }

                        info!(target: "SOLANA", "Subscribed to pump.fun program logs");

                        // Keepalive ping task
                        let msg_count = Arc::clone(&message_count);
                        let ping_task = tokio::spawn(async move {
                            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
                            loop {
                                interval.tick().await;
                                let count = msg_count.load(Ordering::SeqCst);
                                info!(target: "SOLANA", "WebSocket keepalive - {} messages received", count);
                            }
                        });

                        // Message handling loop
                        let mut last_message_time = std::time::Instant::now();
                        
                        loop {
                            // Use timeout to detect stale connections
                            let msg = tokio::time::timeout(
                                tokio::time::Duration::from_secs(120), // 2 minute timeout
                                read.next()
                            ).await;

                            match msg {
                                Ok(Some(Ok(Message::Text(text)))) => {
                                    last_message_time = std::time::Instant::now();
                                    
                                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                        // Check for subscription confirmation
                                        if json.get("result").is_some() {
                                            info!(target: "SOLANA", "Subscription confirmed");
                                            continue;
                                        }
                                        
                                        // Check for errors
                                        if let Some(error) = json.get("error") {
                                            error!(target: "SOLANA", "RPC error: {:?}", error);
                                            continue;
                                        }

                                        if let Some(result) = json.get("params").and_then(|p| p.get("result")) {
                                            if let Some(value) = result.get("value") {
                                                let signature = value
                                                    .get("signature")
                                                    .and_then(|s| s.as_str())
                                                    .unwrap_or("")
                                                    .to_string();

                                                let logs: Vec<String> = value
                                                    .get("logs")
                                                    .and_then(|l| l.as_array())
                                                    .map(|arr| {
                                                        arr.iter()
                                                            .filter_map(|v| v.as_str().map(String::from))
                                                            .collect()
                                                    })
                                                    .unwrap_or_default();

                                                if !signature.is_empty() {
                                                    message_count.fetch_add(1, Ordering::SeqCst);
                                                    let _ = sender.send(LogEvent { signature, logs });
                                                }
                                            }
                                        }
                                    }
                                }
                                Ok(Some(Ok(Message::Ping(data)))) => {
                                    last_message_time = std::time::Instant::now();
                                    // Note: pong is sent via write half, but we're in read half
                                    // Most WS implementations auto-respond to pings
                                }
                                Ok(Some(Ok(Message::Pong(_)))) => {
                                    last_message_time = std::time::Instant::now();
                                }
                                Ok(Some(Ok(Message::Close(frame)))) => {
                                    warn!(target: "SOLANA", "WebSocket closed by server: {:?}", frame);
                                    break;
                                }
                                Ok(Some(Err(e))) => {
                                    error!(target: "SOLANA", "WebSocket error: {}", e);
                                    break;
                                }
                                Ok(None) => {
                                    warn!(target: "SOLANA", "WebSocket stream ended");
                                    break;
                                }
                                Err(_) => {
                                    // Timeout - check if connection is stale
                                    if last_message_time.elapsed() > tokio::time::Duration::from_secs(120) {
                                        warn!(target: "SOLANA", "WebSocket connection stale (no messages for 2 min), reconnecting...");
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }

                        ping_task.abort();
                    }
                    Err(e) => {
                        error!(target: "SOLANA", "Failed to connect WebSocket: {}", e);
                    }
                }

                // Wait before reconnecting with exponential backoff (max 60s)
                info!(target: "SOLANA", "Reconnecting WebSocket in {} seconds...", reconnect_delay);
                tokio::time::sleep(tokio::time::Duration::from_secs(reconnect_delay)).await;
                reconnect_delay = (reconnect_delay * 2).min(60);
            }
        });

        Ok(())
    }

    /// Get account balance in SOL
    pub async fn get_balance(&self, pubkey: &str) -> Result<f64> {
        let pubkey = Pubkey::from_str(pubkey)?;
        let balance = self.client.get_balance(&pubkey).await?;
        Ok(balance as f64 / 1_000_000_000.0)
    }

    /// Get a parsed transaction by signature with retry logic
    pub async fn get_transaction(&self, signature: &str) -> Result<Option<EncodedConfirmedTransactionWithStatusMeta>> {
        let sig = Signature::from_str(signature)?;
        let config = RpcTransactionConfig {
            encoding: Some(UiTransactionEncoding::JsonParsed),
            commitment: Some(CommitmentConfig::confirmed()),
            max_supported_transaction_version: Some(0),
        };

        // Retry with exponential backoff for rate limiting
        let mut attempts = 0;
        let max_attempts = 3;
        let mut delay_ms = 500;

        loop {
            match self.client.get_transaction_with_config(&sig, config.clone()).await {
                Ok(tx) => return Ok(Some(tx)),
                Err(e) => {
                    let error_str = e.to_string();
                    
                    // Check if rate limited (429)
                    if error_str.contains("429") && attempts < max_attempts {
                        attempts += 1;
                        warn!(target: "SOLANA", "Rate limited, retrying in {}ms (attempt {}/{})", delay_ms, attempts, max_attempts);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                        delay_ms *= 2; // Exponential backoff
                        continue;
                    }
                    
                    warn!(target: "SOLANA", "Failed to get transaction {}: {}", signature, e);
                    return Ok(None);
                }
            }
        }
    }

    /// Shorten an address for display
    pub fn shorten_address(address: &str, chars: usize) -> String {
        if address.len() <= chars * 2 {
            return address.to_string();
        }
        format!("{}...{}", &address[..chars], &address[address.len() - chars..])
    }

    /// Derive bonding curve PDA for a token mint
    pub fn derive_bonding_curve(&self, mint: &Pubkey) -> Pubkey {
        let seeds = &[b"bonding-curve", mint.as_ref()];
        let (pda, _) = Pubkey::find_program_address(seeds, &self.pump_program_id);
        pda
    }
}

