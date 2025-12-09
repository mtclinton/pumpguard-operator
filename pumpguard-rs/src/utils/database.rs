//! SQLite database service for PumpGuard

use anyhow::Result;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tracing::info;

/// Token information stored in database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub creator: String,
    pub created_at: String,
    pub initial_liquidity: f64,
    pub current_liquidity: f64,
    pub holder_count: i32,
    pub is_rugged: bool,
    pub rug_reason: Option<String>,
    pub last_updated: String,
}

/// Transaction record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub signature: String,
    pub mint: String,
    pub wallet: String,
    pub tx_type: String,
    pub amount_sol: f64,
    pub amount_tokens: f64,
    pub timestamp: String,
}

/// Wallet record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletRecord {
    pub address: String,
    pub label: String,
    pub total_volume_sol: f64,
    pub last_activity: Option<String>,
    pub is_whale: bool,
}

/// Alert record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRecord {
    pub id: i64,
    pub alert_type: String,
    pub title: String,
    pub message: String,
    pub data: String,
    pub created_at: String,
}

/// Database statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbStats {
    pub total_tokens: i64,
    pub rugged_tokens: i64,
    pub whales: i64,
    pub alerts: i64,
}

/// SQLite database service
pub struct DatabaseService {
    conn: Arc<Mutex<Connection>>,
}

impl DatabaseService {
    /// Create a new database service
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        let service = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        service.initialize()?;
        Ok(service)
    }

    fn initialize(&self) -> Result<()> {
        let conn = self.conn.lock();

        // Tokens table
        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS tokens (
                mint TEXT PRIMARY KEY,
                name TEXT,
                symbol TEXT,
                creator TEXT,
                created_at TEXT,
                initial_liquidity REAL,
                current_liquidity REAL,
                holder_count INTEGER DEFAULT 0,
                is_rugged INTEGER DEFAULT 0,
                rug_reason TEXT,
                last_updated TEXT
            )
            "#,
            [],
        )?;

        // Transactions table
        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS transactions (
                signature TEXT PRIMARY KEY,
                mint TEXT,
                wallet TEXT,
                type TEXT,
                amount_sol REAL,
                amount_tokens REAL,
                timestamp TEXT,
                FOREIGN KEY (mint) REFERENCES tokens(mint)
            )
            "#,
            [],
        )?;

        // Watched wallets (whales)
        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS watched_wallets (
                address TEXT PRIMARY KEY,
                label TEXT,
                total_volume_sol REAL DEFAULT 0,
                last_activity TEXT,
                is_whale INTEGER DEFAULT 0
            )
            "#,
            [],
        )?;

        // Alerts history
        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT,
                title TEXT,
                message TEXT,
                data TEXT,
                created_at TEXT
            )
            "#,
            [],
        )?;

        // Create indexes
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tokens_created ON tokens(created_at)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tx_mint ON transactions(mint)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tx_wallet ON transactions(wallet)",
            [],
        )?;

        info!(target: "DATABASE", "Initialized successfully");
        Ok(())
    }

    // ============================================
    // TOKEN METHODS
    // ============================================

    pub fn save_token(&self, token: &TokenRecord) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT OR REPLACE INTO tokens 
            (mint, name, symbol, creator, created_at, initial_liquidity, current_liquidity, holder_count, last_updated)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                token.mint,
                token.name,
                token.symbol,
                token.creator,
                token.created_at,
                token.initial_liquidity,
                token.current_liquidity,
                token.holder_count,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn get_token(&self, mint: &str) -> Result<Option<TokenRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT * FROM tokens WHERE mint = ?")?;
        let mut rows = stmt.query(params![mint])?;

        if let Some(row) = rows.next()? {
            Ok(Some(TokenRecord {
                mint: row.get(0)?,
                name: row.get(1)?,
                symbol: row.get(2)?,
                creator: row.get(3)?,
                created_at: row.get(4)?,
                initial_liquidity: row.get(5)?,
                current_liquidity: row.get(6)?,
                holder_count: row.get(7)?,
                is_rugged: row.get::<_, i32>(8)? != 0,
                rug_reason: row.get(9)?,
                last_updated: row.get(10)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_recent_tokens(&self, limit: i64) -> Result<Vec<TokenRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT * FROM tokens ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(TokenRecord {
                mint: row.get(0)?,
                name: row.get(1)?,
                symbol: row.get(2)?,
                creator: row.get(3)?,
                created_at: row.get(4)?,
                initial_liquidity: row.get(5)?,
                current_liquidity: row.get(6)?,
                holder_count: row.get(7)?,
                is_rugged: row.get::<_, i32>(8)? != 0,
                rug_reason: row.get(9)?,
                last_updated: row.get(10)?,
            })
        })?;

        let mut tokens = Vec::new();
        for row in rows {
            tokens.push(row?);
        }
        Ok(tokens)
    }

    pub fn mark_as_rugged(&self, mint: &str, reason: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE tokens SET is_rugged = 1, rug_reason = ? WHERE mint = ?",
            params![reason, mint],
        )?;
        Ok(())
    }

    // ============================================
    // TRANSACTION METHODS
    // ============================================

    pub fn save_transaction(&self, tx: &TransactionRecord) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT OR IGNORE INTO transactions 
            (signature, mint, wallet, type, amount_sol, amount_tokens, timestamp)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                tx.signature,
                tx.mint,
                tx.wallet,
                tx.tx_type,
                tx.amount_sol,
                tx.amount_tokens,
                tx.timestamp,
            ],
        )?;
        Ok(())
    }

    pub fn get_transactions_for_token(&self, mint: &str, limit: i64) -> Result<Vec<TransactionRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT * FROM transactions WHERE mint = ? ORDER BY timestamp DESC LIMIT ?",
        )?;
        let rows = stmt.query_map(params![mint, limit], |row| {
            Ok(TransactionRecord {
                signature: row.get(0)?,
                mint: row.get(1)?,
                wallet: row.get(2)?,
                tx_type: row.get(3)?,
                amount_sol: row.get(4)?,
                amount_tokens: row.get(5)?,
                timestamp: row.get(6)?,
            })
        })?;

        let mut txs = Vec::new();
        for row in rows {
            txs.push(row?);
        }
        Ok(txs)
    }

    // ============================================
    // WALLET METHODS
    // ============================================

    pub fn save_wallet(&self, wallet: &WalletRecord) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT OR REPLACE INTO watched_wallets 
            (address, label, total_volume_sol, last_activity, is_whale)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                wallet.address,
                wallet.label,
                wallet.total_volume_sol,
                Utc::now().to_rfc3339(),
                wallet.is_whale as i32,
            ],
        )?;
        Ok(())
    }

    pub fn get_whales(&self) -> Result<Vec<WalletRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT * FROM watched_wallets WHERE is_whale = 1")?;
        let rows = stmt.query_map([], |row| {
            Ok(WalletRecord {
                address: row.get(0)?,
                label: row.get(1)?,
                total_volume_sol: row.get(2)?,
                last_activity: row.get(3)?,
                is_whale: row.get::<_, i32>(4)? != 0,
            })
        })?;

        let mut wallets = Vec::new();
        for row in rows {
            wallets.push(row?);
        }
        Ok(wallets)
    }

    // ============================================
    // ALERT METHODS
    // ============================================

    pub fn save_alert(&self, alert_type: &str, title: &str, message: &str, data: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT INTO alerts (type, title, message, data, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![alert_type, title, message, data, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn get_recent_alerts(&self, limit: i64) -> Result<Vec<AlertRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT * FROM alerts ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(AlertRecord {
                id: row.get(0)?,
                alert_type: row.get(1)?,
                title: row.get(2)?,
                message: row.get(3)?,
                data: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;

        let mut alerts = Vec::new();
        for row in rows {
            alerts.push(row?);
        }
        Ok(alerts)
    }

    // ============================================
    // STATS
    // ============================================

    pub fn get_stats(&self) -> Result<DbStats> {
        let conn = self.conn.lock();
        
        let total_tokens: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tokens",
            [],
            |row| row.get(0),
        )?;

        let rugged_tokens: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tokens WHERE is_rugged = 1",
            [],
            |row| row.get(0),
        )?;

        let whales: i64 = conn.query_row(
            "SELECT COUNT(*) FROM watched_wallets WHERE is_whale = 1",
            [],
            |row| row.get(0),
        )?;

        let alerts: i64 = conn.query_row(
            "SELECT COUNT(*) FROM alerts",
            [],
            |row| row.get(0),
        )?;

        Ok(DbStats {
            total_tokens,
            rugged_tokens,
            whales,
            alerts,
        })
    }
}

impl Clone for DatabaseService {
    fn clone(&self) -> Self {
        Self {
            conn: Arc::clone(&self.conn),
        }
    }
}


