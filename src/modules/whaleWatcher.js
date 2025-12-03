import { PublicKey, LAMPORTS_PER_SOL } from '@solana/web3.js';
import config from '../config.js';
import logger from '../utils/logger.js';
import solanaService from '../utils/solana.js';
import alertService from '../utils/alerts.js';
import databaseService from '../utils/database.js';

class WhaleWatcher {
  constructor() {
    this.isRunning = false;
    this.watchedWallets = new Map();
    this.tokenMovements = new Map();
    this.subscriptionId = null;
    this.scanInterval = null;
    
    this.thresholds = {
      whaleThresholdSol: config.whaleThresholdSol,
      alertOnAccumulation: config.alertOnAccumulation,
      alertOnDump: config.alertOnDump,
      accumulationWindowMs: 3600000, // 1 hour
      minTransactionsForPattern: 3
    };
    
    this.stats = {
      walletsTracked: 0,
      whalesIdentified: 0,
      accumulationAlerts: 0,
      dumpAlerts: 0,
      totalVolumeTracked: 0
    };
  }
  
  async start() {
    if (this.isRunning) {
      logger.warn('WHALE_WATCHER', 'Already running');
      return;
    }
    
    this.isRunning = true;
    logger.whale('Starting Whale Watcher...');
    
    // Load known whales from database
    await this.loadKnownWhales();
    
    // Subscribe to pump.fun logs
    this.subscriptionId = await solanaService.subscribeToPumpLogs(
      (logs) => this.handleLogs(logs)
    );
    
    // Start periodic whale pattern analysis
    this.scanInterval = setInterval(() => this.analyzePatterns(), 60000);
    
    logger.whale('Whale Watcher active - tracking large wallet movements');
  }
  
  async stop() {
    if (!this.isRunning) return;
    
    this.isRunning = false;
    
    if (this.subscriptionId) {
      await solanaService.connection.removeOnLogsListener(this.subscriptionId);
      this.subscriptionId = null;
    }
    
    if (this.scanInterval) {
      clearInterval(this.scanInterval);
      this.scanInterval = null;
    }
    
    logger.whale('Whale Watcher stopped');
  }
  
  async loadKnownWhales() {
    try {
      const whales = databaseService.getWhales();
      for (const whale of whales) {
        this.watchedWallets.set(whale.address, {
          address: whale.address,
          label: whale.label,
          totalVolume: whale.total_volume_sol || 0,
          isWhale: true,
          transactions: [],
          lastActivity: whale.last_activity
        });
      }
      logger.info('WHALE_WATCHER', `Loaded ${whales.length} known whales`);
    } catch (e) {
      logger.error('WHALE_WATCHER', 'Error loading whales', e.message);
    }
  }
  
  watchWallet(address, label = '') {
    if (this.watchedWallets.has(address)) {
      logger.info('WHALE_WATCHER', `Already watching: ${solanaService.shortenAddress(address)}`);
      return;
    }
    
    this.watchedWallets.set(address, {
      address,
      label,
      totalVolume: 0,
      isWhale: false,
      transactions: [],
      lastActivity: null
    });
    
    databaseService.saveWallet({ address, label, isWhale: false });
    this.stats.walletsTracked++;
    
    logger.whale(`Now watching wallet: ${label || solanaService.shortenAddress(address)}`);
  }
  
  unwatchWallet(address) {
    this.watchedWallets.delete(address);
    logger.info('WHALE_WATCHER', `Stopped watching: ${solanaService.shortenAddress(address)}`);
  }
  
  async handleLogs(logs) {
    try {
      const { signature, logs: logMessages } = logs;
      
      // Check for buy/sell events
      const isBuy = logMessages.some(log => log.includes('Program log: Instruction: Buy'));
      const isSell = logMessages.some(log => log.includes('Program log: Instruction: Sell'));
      
      if (isBuy || isSell) {
        await this.analyzeTransaction(signature, isBuy ? 'buy' : 'sell');
      }
      
    } catch (e) {
      logger.error('WHALE_WATCHER', 'Error handling logs', e.message);
    }
  }
  
  async analyzeTransaction(signature, type) {
    try {
      await new Promise(resolve => setTimeout(resolve, 300));
      
      const tx = await solanaService.getTransaction(signature);
      if (!tx) return;
      
      const txInfo = this.parseTransaction(tx, type);
      if (!txInfo) return;
      
      // Check if this is a whale transaction
      if (txInfo.amountSol >= this.thresholds.whaleThresholdSol) {
        await this.handleWhaleTransaction(txInfo);
      }
      
      // Track wallet if not already tracked
      this.trackWalletActivity(txInfo);
      
      // Track token movements
      this.trackTokenMovement(txInfo);
      
    } catch (e) {
      logger.error('WHALE_WATCHER', 'Error analyzing transaction', e.message);
    }
  }
  
  parseTransaction(tx, type) {
    try {
      const accounts = tx.transaction.message.accountKeys;
      let wallet = null;
      let mint = null;
      let amountSol = 0;
      let amountTokens = 0;
      
      // Get wallet
      for (const account of accounts) {
        if (account.signer) {
          wallet = account.pubkey.toBase58();
          break;
        }
      }
      
      // Get mint from token balances
      if (tx.meta?.preTokenBalances?.length > 0) {
        mint = tx.meta.preTokenBalances[0].mint;
      } else if (tx.meta?.postTokenBalances?.length > 0) {
        mint = tx.meta.postTokenBalances[0].mint;
      }
      
      // Calculate SOL amount
      if (tx.meta?.preBalances && tx.meta?.postBalances) {
        const solChange = Math.abs(tx.meta.postBalances[0] - tx.meta.preBalances[0]);
        amountSol = solChange / LAMPORTS_PER_SOL;
      }
      
      // Calculate token amount
      const walletTokenBalances = (type === 'buy' ? tx.meta?.postTokenBalances : tx.meta?.preTokenBalances) || [];
      const balance = walletTokenBalances.find(b => b.owner === wallet);
      if (balance) {
        amountTokens = balance.uiTokenAmount?.uiAmount || 0;
      }
      
      if (!mint || !wallet) return null;
      
      return {
        signature: tx.transaction.signatures[0],
        wallet,
        mint,
        type,
        amountSol,
        amountTokens,
        timestamp: Date.now()
      };
      
    } catch (e) {
      return null;
    }
  }
  
  async handleWhaleTransaction(txInfo) {
    const { wallet, mint, type, amountSol, amountTokens } = txInfo;
    
    // Get or create wallet entry
    let walletData = this.watchedWallets.get(wallet);
    if (!walletData) {
      walletData = {
        address: wallet,
        label: '',
        totalVolume: 0,
        isWhale: true,
        transactions: [],
        lastActivity: null
      };
      this.watchedWallets.set(wallet, walletData);
      this.stats.whalesIdentified++;
    }
    
    // Mark as whale
    if (!walletData.isWhale) {
      walletData.isWhale = true;
      this.stats.whalesIdentified++;
      logger.whale(`New whale identified: ${solanaService.shortenAddress(wallet)}`);
    }
    
    // Update wallet data
    walletData.totalVolume += amountSol;
    walletData.lastActivity = new Date().toISOString();
    walletData.transactions.push(txInfo);
    
    // Keep only recent transactions
    if (walletData.transactions.length > 100) {
      walletData.transactions = walletData.transactions.slice(-50);
    }
    
    // Save to database
    databaseService.saveWallet({
      address: wallet,
      label: walletData.label,
      totalVolumeSol: walletData.totalVolume,
      isWhale: true
    });
    
    databaseService.saveTransaction({
      signature: txInfo.signature,
      mint,
      wallet,
      type,
      amountSol,
      amountTokens
    });
    
    // Get token info
    const tokenInfo = databaseService.getToken(mint) || { symbol: 'UNKNOWN', mint };
    
    // Log and alert
    if (type === 'buy') {
      logger.whale(`Whale BUYING: ${amountSol.toFixed(2)} SOL of ${tokenInfo.symbol}`, {
        wallet: solanaService.shortenAddress(wallet)
      });
      
      if (this.thresholds.alertOnAccumulation) {
        this.stats.accumulationAlerts++;
        await alertService.alertWhale('buy', wallet, tokenInfo, amountSol, amountTokens);
      }
    } else {
      logger.whale(`Whale SELLING: ${amountSol.toFixed(2)} SOL of ${tokenInfo.symbol}`, {
        wallet: solanaService.shortenAddress(wallet)
      });
      
      if (this.thresholds.alertOnDump) {
        this.stats.dumpAlerts++;
        await alertService.alertWhale('sell', wallet, tokenInfo, amountSol, amountTokens);
      }
    }
    
    this.stats.totalVolumeTracked += amountSol;
    this.watchedWallets.set(wallet, walletData);
  }
  
  trackWalletActivity(txInfo) {
    const { wallet, amountSol } = txInfo;
    
    let walletData = this.watchedWallets.get(wallet);
    if (!walletData) {
      walletData = {
        address: wallet,
        label: '',
        totalVolume: 0,
        isWhale: false,
        transactions: [],
        lastActivity: null
      };
    }
    
    walletData.totalVolume += amountSol;
    walletData.lastActivity = new Date().toISOString();
    walletData.transactions.push(txInfo);
    
    // Check if wallet has become a whale
    if (!walletData.isWhale && walletData.totalVolume >= this.thresholds.whaleThresholdSol * 2) {
      walletData.isWhale = true;
      this.stats.whalesIdentified++;
      logger.whale(`Wallet promoted to whale status: ${solanaService.shortenAddress(wallet)} (${walletData.totalVolume.toFixed(2)} SOL volume)`);
    }
    
    this.watchedWallets.set(wallet, walletData);
  }
  
  trackTokenMovement(txInfo) {
    const { mint, type, amountSol, amountTokens, wallet, timestamp } = txInfo;
    
    let tokenData = this.tokenMovements.get(mint);
    if (!tokenData) {
      tokenData = {
        mint,
        buys: [],
        sells: [],
        netFlow: 0,
        uniqueBuyers: new Set(),
        uniqueSellers: new Set()
      };
    }
    
    const movement = { wallet, amountSol, amountTokens, timestamp };
    
    if (type === 'buy') {
      tokenData.buys.push(movement);
      tokenData.netFlow += amountSol;
      tokenData.uniqueBuyers.add(wallet);
    } else {
      tokenData.sells.push(movement);
      tokenData.netFlow -= amountSol;
      tokenData.uniqueSellers.add(wallet);
    }
    
    // Keep only recent data
    const cutoff = Date.now() - this.thresholds.accumulationWindowMs;
    tokenData.buys = tokenData.buys.filter(m => m.timestamp > cutoff);
    tokenData.sells = tokenData.sells.filter(m => m.timestamp > cutoff);
    
    this.tokenMovements.set(mint, tokenData);
  }
  
  async analyzePatterns() {
    // Analyze accumulation patterns
    for (const [mint, data] of this.tokenMovements) {
      try {
        // Check for whale accumulation pattern
        const whaleBuys = data.buys.filter(b => b.amountSol >= this.thresholds.whaleThresholdSol);
        
        if (whaleBuys.length >= this.thresholds.minTransactionsForPattern) {
          const totalAccumulation = whaleBuys.reduce((sum, b) => sum + b.amountSol, 0);
          const tokenInfo = databaseService.getToken(mint) || { symbol: 'UNKNOWN', mint };
          
          logger.whale(`Accumulation pattern detected for ${tokenInfo.symbol}: ${whaleBuys.length} whale buys totaling ${totalAccumulation.toFixed(2)} SOL`);
        }
        
        // Check for coordinated selling
        const whaleSells = data.sells.filter(s => s.amountSol >= this.thresholds.whaleThresholdSol);
        
        if (whaleSells.length >= this.thresholds.minTransactionsForPattern) {
          const totalDump = whaleSells.reduce((sum, s) => sum + s.amountSol, 0);
          const tokenInfo = databaseService.getToken(mint) || { symbol: 'UNKNOWN', mint };
          
          logger.warn('WHALE_WATCHER', `Dump pattern detected for ${tokenInfo.symbol}: ${whaleSells.length} whale sells totaling ${totalDump.toFixed(2)} SOL`);
        }
        
      } catch (e) {
        logger.error('WHALE_WATCHER', 'Pattern analysis error', e.message);
      }
    }
    
    // Clean up old data
    const cutoff = Date.now() - this.thresholds.accumulationWindowMs * 2;
    for (const [mint, data] of this.tokenMovements) {
      if (data.buys.length === 0 && data.sells.length === 0) {
        this.tokenMovements.delete(mint);
      }
    }
  }
  
  getStats() {
    return {
      ...this.stats,
      watchedWallets: this.watchedWallets.size,
      tokensTracked: this.tokenMovements.size,
      isRunning: this.isRunning
    };
  }
  
  getWhales() {
    return Array.from(this.watchedWallets.values())
      .filter(w => w.isWhale)
      .map(w => ({
        address: w.address,
        label: w.label,
        totalVolume: w.totalVolume,
        lastActivity: w.lastActivity,
        recentTransactions: w.transactions.slice(-10)
      }))
      .sort((a, b) => b.totalVolume - a.totalVolume);
  }
  
  getWalletActivity(address) {
    return this.watchedWallets.get(address);
  }
  
  getTokenFlow(mint) {
    const data = this.tokenMovements.get(mint);
    if (!data) return null;
    
    return {
      mint,
      netFlow: data.netFlow,
      buyCount: data.buys.length,
      sellCount: data.sells.length,
      uniqueBuyers: data.uniqueBuyers.size,
      uniqueSellers: data.uniqueSellers.size,
      totalBuyVolume: data.buys.reduce((sum, b) => sum + b.amountSol, 0),
      totalSellVolume: data.sells.reduce((sum, s) => sum + s.amountSol, 0)
    };
  }
  
  getTopMovers(limit = 10) {
    return Array.from(this.tokenMovements.entries())
      .map(([mint, data]) => ({
        mint,
        netFlow: data.netFlow,
        volume: data.buys.reduce((s, b) => s + b.amountSol, 0) + 
                data.sells.reduce((s, b) => s + b.amountSol, 0)
      }))
      .sort((a, b) => Math.abs(b.netFlow) - Math.abs(a.netFlow))
      .slice(0, limit);
  }
}

export const whaleWatcher = new WhaleWatcher();
export default whaleWatcher;

