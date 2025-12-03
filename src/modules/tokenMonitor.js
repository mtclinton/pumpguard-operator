import { PublicKey, LAMPORTS_PER_SOL } from '@solana/web3.js';
import config from '../config.js';
import logger from '../utils/logger.js';
import solanaService from '../utils/solana.js';
import alertService from '../utils/alerts.js';
import databaseService from '../utils/database.js';

class TokenMonitor {
  constructor() {
    this.isRunning = false;
    this.subscriptionId = null;
    this.detectedTokens = new Map();
    this.filters = {
      minLiquiditySol: 0,
      maxLiquiditySol: Infinity,
      blacklistedCreators: new Set(),
      whitelistedCreators: new Set()
    };
    this.stats = {
      tokensDetected: 0,
      alertsSent: 0
    };
  }
  
  async start() {
    if (this.isRunning) {
      logger.warn('TOKEN_MONITOR', 'Already running');
      return;
    }
    
    this.isRunning = true;
    logger.info('TOKEN_MONITOR', '🆕 Starting Token Monitor...');
    
    // Subscribe to pump.fun program logs
    this.subscriptionId = await solanaService.subscribeToPumpLogs(
      (logs) => this.handleLogs(logs)
    );
    
    logger.success('TOKEN_MONITOR', 'Token Monitor active - watching for new token launches');
  }
  
  async stop() {
    if (!this.isRunning) return;
    
    this.isRunning = false;
    if (this.subscriptionId) {
      await solanaService.connection.removeOnLogsListener(this.subscriptionId);
      this.subscriptionId = null;
    }
    
    logger.info('TOKEN_MONITOR', 'Token Monitor stopped');
  }
  
  async handleLogs(logs) {
    try {
      const { signature, logs: logMessages } = logs;
      
      // Check for token creation
      const isCreate = logMessages.some(log => 
        log.includes('Program log: Instruction: Create') ||
        log.includes('Program log: Instruction: Initialize')
      );
      
      if (isCreate) {
        await this.handleNewToken(signature);
      }
    } catch (e) {
      logger.error('TOKEN_MONITOR', 'Error handling logs', e.message);
    }
  }
  
  async handleNewToken(signature) {
    try {
      // Small delay to ensure transaction is confirmed
      await new Promise(resolve => setTimeout(resolve, 500));
      
      const tx = await solanaService.getTransaction(signature);
      if (!tx) return;
      
      const tokenInfo = this.parseTokenCreation(tx);
      if (!tokenInfo) return;
      
      // Check filters
      if (!this.passesFilters(tokenInfo)) return;
      
      this.stats.tokensDetected++;
      
      logger.info('TOKEN_MONITOR', `🆕 New token: ${tokenInfo.name} (${tokenInfo.symbol})`, {
        mint: solanaService.shortenAddress(tokenInfo.mint),
        creator: solanaService.shortenAddress(tokenInfo.creator),
        liquidity: `${tokenInfo.initialLiquidity.toFixed(2)} SOL`
      });
      
      // Save to database
      databaseService.saveToken(tokenInfo);
      
      // Store in memory
      this.detectedTokens.set(tokenInfo.mint, {
        ...tokenInfo,
        detectedAt: Date.now()
      });
      
      // Keep only last 1000 tokens in memory
      if (this.detectedTokens.size > 1000) {
        const oldest = Array.from(this.detectedTokens.keys())[0];
        this.detectedTokens.delete(oldest);
      }
      
      // Send alert
      this.stats.alertsSent++;
      await alertService.alertNewToken(tokenInfo);
      
      return tokenInfo;
      
    } catch (e) {
      logger.error('TOKEN_MONITOR', 'Error handling new token', e.message);
    }
  }
  
  parseTokenCreation(tx) {
    try {
      const accounts = tx.transaction.message.accountKeys;
      
      // Find the mint account and creator
      let mint = null;
      let creator = null;
      let name = 'Unknown';
      let symbol = 'UNK';
      
      // Parse from account keys
      for (const account of accounts) {
        if (account.signer && account.writable) {
          creator = account.pubkey.toBase58();
          break;
        }
      }
      
      // Try to find mint from inner instructions or logs
      if (tx.meta?.innerInstructions) {
        for (const inner of tx.meta.innerInstructions) {
          for (const inst of inner.instructions) {
            if (inst.parsed?.type === 'initializeMint') {
              mint = inst.parsed.info.mint;
            }
          }
        }
      }
      
      // Parse token metadata from logs
      if (tx.meta?.logMessages) {
        for (const log of tx.meta.logMessages) {
          if (log.includes('name:')) {
            const match = log.match(/name:\s*([^,]+)/);
            if (match) name = match[1].trim();
          }
          if (log.includes('symbol:')) {
            const match = log.match(/symbol:\s*([^,]+)/);
            if (match) symbol = match[1].trim();
          }
        }
      }
      
      // Fallback: get mint from post token balances
      if (!mint && tx.meta?.postTokenBalances?.length > 0) {
        mint = tx.meta.postTokenBalances[0].mint;
      }
      
      if (!mint) return null;
      
      return {
        mint,
        name,
        symbol,
        creator,
        createdAt: new Date().toISOString(),
        signature: tx.transaction.signatures[0],
        initialLiquidity: this.calculateInitialLiquidity(tx)
      };
      
    } catch (e) {
      logger.error('TOKEN_MONITOR', 'Error parsing token creation', e.message);
      return null;
    }
  }
  
  calculateInitialLiquidity(tx) {
    try {
      // Calculate SOL moved in the transaction
      if (tx.meta?.preBalances && tx.meta?.postBalances) {
        const diff = tx.meta.preBalances[0] - tx.meta.postBalances[0];
        return Math.abs(diff) / LAMPORTS_PER_SOL;
      }
      return 0;
    } catch (e) {
      return 0;
    }
  }
  
  passesFilters(tokenInfo) {
    // Check blacklist
    if (this.filters.blacklistedCreators.has(tokenInfo.creator)) {
      return false;
    }
    
    // Check whitelist (if not empty, only allow whitelisted)
    if (this.filters.whitelistedCreators.size > 0) {
      if (!this.filters.whitelistedCreators.has(tokenInfo.creator)) {
        return false;
      }
    }
    
    // Check liquidity bounds
    if (tokenInfo.initialLiquidity < this.filters.minLiquiditySol) {
      return false;
    }
    
    if (tokenInfo.initialLiquidity > this.filters.maxLiquiditySol) {
      return false;
    }
    
    return true;
  }
  
  // Configuration methods
  setFilter(key, value) {
    if (key in this.filters) {
      this.filters[key] = value;
      logger.info('TOKEN_MONITOR', `Filter updated: ${key} = ${value}`);
    }
  }
  
  blacklistCreator(address) {
    this.filters.blacklistedCreators.add(address);
    logger.info('TOKEN_MONITOR', `Creator blacklisted: ${address}`);
  }
  
  whitelistCreator(address) {
    this.filters.whitelistedCreators.add(address);
    logger.info('TOKEN_MONITOR', `Creator whitelisted: ${address}`);
  }
  
  getStats() {
    return {
      ...this.stats,
      tokensTracked: this.detectedTokens.size,
      isRunning: this.isRunning
    };
  }
  
  getRecentTokens(limit = 50) {
    return Array.from(this.detectedTokens.values())
      .sort((a, b) => b.detectedAt - a.detectedAt)
      .slice(0, limit);
  }
  
  getToken(mint) {
    return this.detectedTokens.get(mint);
  }
}

export const tokenMonitor = new TokenMonitor();
export default tokenMonitor;

