import Database from 'better-sqlite3';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';
import logger from './logger.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

class DatabaseService {
  constructor() {
    const dbPath = join(__dirname, '../../data/pumpguard.db');
    this.db = new Database(dbPath);
    this.initialize();
  }
  
  initialize() {
    // Tokens table
    this.db.exec(`
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
    `);
    
    // Transactions table
    this.db.exec(`
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
    `);
    
    // Watched wallets (whales)
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS watched_wallets (
        address TEXT PRIMARY KEY,
        label TEXT,
        total_volume_sol REAL DEFAULT 0,
        last_activity TEXT,
        is_whale INTEGER DEFAULT 0
      )
    `);
    
    // Alerts history
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS alerts (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        type TEXT,
        title TEXT,
        message TEXT,
        data TEXT,
        created_at TEXT
      )
    `);
    
    logger.info('DATABASE', 'Initialized successfully');
  }
  
  // Token methods
  saveToken(token) {
    const stmt = this.db.prepare(`
      INSERT OR REPLACE INTO tokens (mint, name, symbol, creator, created_at, initial_liquidity, current_liquidity, holder_count, last_updated)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    stmt.run(
      token.mint,
      token.name,
      token.symbol,
      token.creator,
      token.createdAt || new Date().toISOString(),
      token.initialLiquidity || 0,
      token.currentLiquidity || 0,
      token.holderCount || 0,
      new Date().toISOString()
    );
  }
  
  getToken(mint) {
    return this.db.prepare('SELECT * FROM tokens WHERE mint = ?').get(mint);
  }
  
  getRecentTokens(limit = 50) {
    return this.db.prepare('SELECT * FROM tokens ORDER BY created_at DESC LIMIT ?').all(limit);
  }
  
  markAsRugged(mint, reason) {
    this.db.prepare('UPDATE tokens SET is_rugged = 1, rug_reason = ? WHERE mint = ?').run(reason, mint);
  }
  
  getRuggedTokens(limit = 50) {
    return this.db.prepare('SELECT * FROM tokens WHERE is_rugged = 1 ORDER BY last_updated DESC LIMIT ?').all(limit);
  }
  
  // Transaction methods
  saveTransaction(tx) {
    const stmt = this.db.prepare(`
      INSERT OR IGNORE INTO transactions (signature, mint, wallet, type, amount_sol, amount_tokens, timestamp)
      VALUES (?, ?, ?, ?, ?, ?, ?)
    `);
    stmt.run(tx.signature, tx.mint, tx.wallet, tx.type, tx.amountSol, tx.amountTokens, tx.timestamp || new Date().toISOString());
  }
  
  getTransactionsForToken(mint, limit = 100) {
    return this.db.prepare('SELECT * FROM transactions WHERE mint = ? ORDER BY timestamp DESC LIMIT ?').all(mint, limit);
  }
  
  getTransactionsForWallet(wallet, limit = 100) {
    return this.db.prepare('SELECT * FROM transactions WHERE wallet = ? ORDER BY timestamp DESC LIMIT ?').all(wallet, limit);
  }
  
  // Wallet methods
  saveWallet(wallet) {
    const stmt = this.db.prepare(`
      INSERT OR REPLACE INTO watched_wallets (address, label, total_volume_sol, last_activity, is_whale)
      VALUES (?, ?, ?, ?, ?)
    `);
    stmt.run(wallet.address, wallet.label || '', wallet.totalVolumeSol || 0, new Date().toISOString(), wallet.isWhale ? 1 : 0);
  }
  
  getWallet(address) {
    return this.db.prepare('SELECT * FROM watched_wallets WHERE address = ?').get(address);
  }
  
  getWhales() {
    return this.db.prepare('SELECT * FROM watched_wallets WHERE is_whale = 1').all();
  }
  
  updateWalletVolume(address, volumeToAdd) {
    this.db.prepare(`
      UPDATE watched_wallets 
      SET total_volume_sol = total_volume_sol + ?, last_activity = ?
      WHERE address = ?
    `).run(volumeToAdd, new Date().toISOString(), address);
  }
  
  // Alert methods
  saveAlert(alert) {
    const stmt = this.db.prepare(`
      INSERT INTO alerts (type, title, message, data, created_at)
      VALUES (?, ?, ?, ?, ?)
    `);
    stmt.run(alert.type, alert.title, alert.message, JSON.stringify(alert.data), new Date().toISOString());
  }
  
  getRecentAlerts(limit = 50) {
    return this.db.prepare('SELECT * FROM alerts ORDER BY created_at DESC LIMIT ?').all(limit);
  }
  
  // Stats
  getStats() {
    const tokenCount = this.db.prepare('SELECT COUNT(*) as count FROM tokens').get().count;
    const ruggedCount = this.db.prepare('SELECT COUNT(*) as count FROM tokens WHERE is_rugged = 1').get().count;
    const whaleCount = this.db.prepare('SELECT COUNT(*) as count FROM watched_wallets WHERE is_whale = 1').get().count;
    const alertCount = this.db.prepare('SELECT COUNT(*) as count FROM alerts').get().count;
    
    return {
      totalTokens: tokenCount,
      ruggedTokens: ruggedCount,
      whales: whaleCount,
      alerts: alertCount
    };
  }
}

export const databaseService = new DatabaseService();
export default databaseService;
