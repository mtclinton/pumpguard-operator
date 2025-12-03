import { PublicKey, LAMPORTS_PER_SOL } from '@solana/web3.js';
import config from '../config.js';
import logger from '../utils/logger.js';
import solanaService from '../utils/solana.js';
import alertService from '../utils/alerts.js';
import databaseService from '../utils/database.js';

class RugDetector {
  constructor() {
    this.isRunning = false;
    this.watchedTokens = new Map();
    this.subscriptionId = null;
    this.checkInterval = null;
    
    this.thresholds = {
      lpRemovalPercent: config.lpRemovalThresholdPercent,
      suspiciousSellPercent: config.suspiciousSellPercent,
      devWalletSellAlert: config.devWalletSellAlert,
      maxDevSellPercent: 20, // Alert if dev sells more than 20% at once
      minTimeBetweenSells: 60000, // 1 minute - rapid selling is suspicious
      holderConcentrationAlert: 80 // Alert if top holder has >80%
    };
    
    this.stats = {
      tokensWatched: 0,
      rugsDetected: 0,
      alertsSent: 0
    };
  }
  
  async start() {
    if (this.isRunning) {
      logger.warn('RUG_DETECTOR', 'Already running');
      return;
    }
    
    this.isRunning = true;
    logger.info('RUG_DETECTOR', '🔍 Starting Rug Pull Detector...');
    
    // Subscribe to pump.fun logs for sell events
    this.subscriptionId = await solanaService.subscribeToPumpLogs(
      (logs) => this.handleLogs(logs)
    );
    
    // Start periodic health checks
    this.checkInterval = setInterval(() => this.performHealthChecks(), 30000);
    
    logger.success('RUG_DETECTOR', 'Rug Pull Detector active - monitoring for suspicious activity');
  }
  
  async stop() {
    if (!this.isRunning) return;
    
    this.isRunning = false;
    
    if (this.subscriptionId) {
      await solanaService.connection.removeOnLogsListener(this.subscriptionId);
      this.subscriptionId = null;
    }
    
    if (this.checkInterval) {
      clearInterval(this.checkInterval);
      this.checkInterval = null;
    }
    
    logger.info('RUG_DETECTOR', 'Rug Pull Detector stopped');
  }
  
  watchToken(tokenInfo) {
    if (this.watchedTokens.has(tokenInfo.mint)) return;
    
    this.watchedTokens.set(tokenInfo.mint, {
      ...tokenInfo,
      initialLiquidity: tokenInfo.initialLiquidity || 0,
      currentLiquidity: tokenInfo.initialLiquidity || 0,
      devWallet: tokenInfo.creator,
      sellHistory: [],
      lastCheck: Date.now(),
      suspicionScore: 0,
      alerts: []
    });
    
    this.stats.tokensWatched++;
    logger.info('RUG_DETECTOR', `Now watching: ${tokenInfo.symbol} (${solanaService.shortenAddress(tokenInfo.mint)})`);
  }
  
  unwatchToken(mint) {
    this.watchedTokens.delete(mint);
    logger.info('RUG_DETECTOR', `Stopped watching: ${solanaService.shortenAddress(mint)}`);
  }
  
  async handleLogs(logs) {
    try {
      const { signature, logs: logMessages } = logs;
      
      // Check for sell events
      const isSell = logMessages.some(log => 
        log.includes('Program log: Instruction: Sell')
      );
      
      if (isSell) {
        await this.analyzeSellTransaction(signature);
      }
      
      // Check for LP removal / migration
      const isLpRemoval = logMessages.some(log =>
        log.includes('withdraw') || 
        log.includes('remove_liquidity') ||
        log.includes('migrate')
      );
      
      if (isLpRemoval) {
        await this.analyzeLpRemoval(signature);
      }
      
    } catch (e) {
      logger.error('RUG_DETECTOR', 'Error handling logs', e.message);
    }
  }
  
  async analyzeSellTransaction(signature) {
    try {
      await new Promise(resolve => setTimeout(resolve, 300));
      
      const tx = await solanaService.getTransaction(signature);
      if (!tx) return;
      
      const sellInfo = this.parseSellTransaction(tx);
      if (!sellInfo) return;
      
      const token = this.watchedTokens.get(sellInfo.mint);
      if (!token) return;
      
      // Record the sell
      token.sellHistory.push({
        signature,
        wallet: sellInfo.wallet,
        amountSol: sellInfo.amountSol,
        amountTokens: sellInfo.amountTokens,
        timestamp: Date.now()
      });
      
      // Save transaction
      databaseService.saveTransaction({
        signature,
        mint: sellInfo.mint,
        wallet: sellInfo.wallet,
        type: 'sell',
        amountSol: sellInfo.amountSol,
        amountTokens: sellInfo.amountTokens
      });
      
      // Check for suspicious patterns
      await this.checkSuspiciousPatterns(token, sellInfo);
      
    } catch (e) {
      logger.error('RUG_DETECTOR', 'Error analyzing sell', e.message);
    }
  }
  
  parseSellTransaction(tx) {
    try {
      const accounts = tx.transaction.message.accountKeys;
      let wallet = null;
      let mint = null;
      let amountSol = 0;
      let amountTokens = 0;
      
      // Get seller wallet
      for (const account of accounts) {
        if (account.signer) {
          wallet = account.pubkey.toBase58();
          break;
        }
      }
      
      // Get mint from token balances
      if (tx.meta?.preTokenBalances?.length > 0) {
        mint = tx.meta.preTokenBalances[0].mint;
      }
      
      // Calculate amounts from balance changes
      if (tx.meta?.preBalances && tx.meta?.postBalances) {
        const solChange = tx.meta.postBalances[0] - tx.meta.preBalances[0];
        amountSol = Math.abs(solChange) / LAMPORTS_PER_SOL;
      }
      
      if (tx.meta?.preTokenBalances && tx.meta?.postTokenBalances) {
        const pre = tx.meta.preTokenBalances.find(b => b.owner === wallet);
        const post = tx.meta.postTokenBalances.find(b => b.owner === wallet);
        if (pre && post) {
          amountTokens = Math.abs(
            (pre.uiTokenAmount?.uiAmount || 0) - (post.uiTokenAmount?.uiAmount || 0)
          );
        }
      }
      
      if (!mint || !wallet) return null;
      
      return { mint, wallet, amountSol, amountTokens, signature: tx.transaction.signatures[0] };
      
    } catch (e) {
      return null;
    }
  }
  
  async checkSuspiciousPatterns(token, sellInfo) {
    const alerts = [];
    let severity = 'low';
    
    // 1. Dev wallet selling
    if (sellInfo.wallet === token.devWallet) {
      const sellPercent = (sellInfo.amountTokens / (token.totalSupply || 1000000000)) * 100;
      
      if (sellPercent >= this.thresholds.maxDevSellPercent) {
        severity = 'critical';
        alerts.push({
          type: 'dev_dump',
          message: `Developer sold ${sellPercent.toFixed(2)}% of supply`,
          severity: 'critical'
        });
        token.suspicionScore += 50;
      } else if (this.thresholds.devWalletSellAlert) {
        alerts.push({
          type: 'dev_sell',
          message: `Developer sold ${sellInfo.amountSol.toFixed(4)} SOL worth`,
          severity: 'medium'
        });
        token.suspicionScore += 20;
      }
    }
    
    // 2. Rapid selling pattern
    const recentSells = token.sellHistory.filter(
      s => Date.now() - s.timestamp < this.thresholds.minTimeBetweenSells
    );
    
    if (recentSells.length >= 3) {
      const totalSoldSol = recentSells.reduce((sum, s) => sum + s.amountSol, 0);
      if (totalSoldSol > token.initialLiquidity * 0.3) {
        severity = 'high';
        alerts.push({
          type: 'rapid_selling',
          message: `Rapid selling detected: ${totalSoldSol.toFixed(2)} SOL in ${recentSells.length} txs`,
          severity: 'high'
        });
        token.suspicionScore += 30;
      }
    }
    
    // 3. Large single sell (whale dump)
    if (sellInfo.amountSol > token.currentLiquidity * (this.thresholds.suspiciousSellPercent / 100)) {
      alerts.push({
        type: 'large_sell',
        message: `Large sell: ${sellInfo.amountSol.toFixed(4)} SOL (${((sellInfo.amountSol / token.currentLiquidity) * 100).toFixed(1)}% of liquidity)`,
        severity: 'medium'
      });
      token.suspicionScore += 15;
    }
    
    // 4. Check if this triggers rug threshold
    if (token.suspicionScore >= 80) {
      severity = 'critical';
      await this.triggerRugAlert(token, 'High suspicion score reached');
    }
    
    // Send alerts
    for (const alert of alerts) {
      token.alerts.push(alert);
      this.stats.alertsSent++;
      
      if (alert.severity === 'critical') {
        logger.rug(`${token.symbol}: ${alert.message}`);
        await alertService.alertRugPull(token, alert.message, alert.severity);
      } else {
        logger.warn('RUG_DETECTOR', `${token.symbol}: ${alert.message}`);
        await alertService.alertSuspicious(token, alert.message);
      }
    }
    
    this.watchedTokens.set(token.mint, token);
  }
  
  async analyzeLpRemoval(signature) {
    try {
      const tx = await solanaService.getTransaction(signature);
      if (!tx) return;
      
      // Check if this affects any watched tokens
      const affectedMints = new Set();
      
      if (tx.meta?.preTokenBalances) {
        for (const balance of tx.meta.preTokenBalances) {
          if (this.watchedTokens.has(balance.mint)) {
            affectedMints.add(balance.mint);
          }
        }
      }
      
      for (const mint of affectedMints) {
        const token = this.watchedTokens.get(mint);
        if (!token) continue;
        
        // Calculate liquidity change
        const preBalance = tx.meta.preBalances[0] || 0;
        const postBalance = tx.meta.postBalances[0] || 0;
        const lpChange = (preBalance - postBalance) / LAMPORTS_PER_SOL;
        
        if (lpChange > token.currentLiquidity * (this.thresholds.lpRemovalPercent / 100)) {
          await this.triggerRugAlert(token, `LP removed: ${lpChange.toFixed(2)} SOL (${((lpChange / token.currentLiquidity) * 100).toFixed(1)}%)`);
        }
      }
      
    } catch (e) {
      logger.error('RUG_DETECTOR', 'Error analyzing LP removal', e.message);
    }
  }
  
  async triggerRugAlert(token, reason) {
    this.stats.rugsDetected++;
    
    logger.rug(`RUG DETECTED: ${token.symbol} - ${reason}`);
    
    // Mark as rugged in database
    databaseService.markAsRugged(token.mint, reason);
    
    // Send critical alert
    await alertService.alertRugPull(token, reason, 'critical');
    
    // Update token status
    token.isRugged = true;
    token.rugReason = reason;
    this.watchedTokens.set(token.mint, token);
  }
  
  async performHealthChecks() {
    for (const [mint, token] of this.watchedTokens) {
      try {
        // Skip if recently checked
        if (Date.now() - token.lastCheck < 25000) continue;
        
        token.lastCheck = Date.now();
        
        // Check liquidity health
        await this.checkLiquidityHealth(token);
        
        this.watchedTokens.set(mint, token);
        
      } catch (e) {
        logger.error('RUG_DETECTOR', `Health check failed for ${token.symbol}`, e.message);
      }
    }
  }
  
  async checkLiquidityHealth(token) {
    try {
      // Get bonding curve balance
      const bondingCurve = this.deriveBondingCurve(new PublicKey(token.mint));
      const balance = await solanaService.getBalance(bondingCurve.toBase58());
      
      const previousLiquidity = token.currentLiquidity;
      token.currentLiquidity = balance;
      
      // Check for significant drop
      if (previousLiquidity > 0) {
        const dropPercent = ((previousLiquidity - balance) / previousLiquidity) * 100;
        
        if (dropPercent >= this.thresholds.lpRemovalPercent) {
          await this.triggerRugAlert(token, `Liquidity dropped ${dropPercent.toFixed(1)}%`);
        }
      }
      
    } catch (e) {
      // Token might not exist anymore
    }
  }
  
  deriveBondingCurve(mint) {
    const [bondingCurve] = PublicKey.findProgramAddressSync(
      [Buffer.from('bonding-curve'), mint.toBuffer()],
      solanaService.pumpProgramId
    );
    return bondingCurve;
  }
  
  getStats() {
    return {
      ...this.stats,
      watchedTokens: this.watchedTokens.size,
      isRunning: this.isRunning
    };
  }
  
  getWatchedTokens() {
    return Array.from(this.watchedTokens.values()).map(t => ({
      mint: t.mint,
      symbol: t.symbol,
      name: t.name,
      suspicionScore: t.suspicionScore,
      currentLiquidity: t.currentLiquidity,
      isRugged: t.isRugged,
      alertCount: t.alerts.length
    }));
  }
  
  getTokenDetails(mint) {
    return this.watchedTokens.get(mint);
  }
}

export const rugDetector = new RugDetector();
export default rugDetector;
