import TelegramBot from 'node-telegram-bot-api';
import config from '../config.js';
import logger from './logger.js';

class AlertService {
  constructor() {
    this.telegramBot = null;
    this.subscribers = new Set();
    this.alertHistory = [];
    
    if (config.telegramBotToken && config.telegramChatId) {
      this.telegramBot = new TelegramBot(config.telegramBotToken, { polling: false });
      logger.info('ALERTS', 'Telegram bot initialized');
    }
  }
  
  subscribe(callback) {
    this.subscribers.add(callback);
    return () => this.subscribers.delete(callback);
  }
  
  async sendAlert(type, title, message, data = {}) {
    const alert = {
      id: Date.now(),
      type,
      title,
      message,
      data,
      timestamp: new Date().toISOString()
    };
    
    this.alertHistory.push(alert);
    if (this.alertHistory.length > 1000) {
      this.alertHistory = this.alertHistory.slice(-500);
    }
    
    // Notify subscribers (for dashboard)
    this.subscribers.forEach(callback => {
      try {
        callback(alert);
      } catch (e) {
        logger.error('ALERTS', 'Subscriber callback error', e.message);
      }
    });
    
    // Send to Telegram
    if (this.telegramBot && config.telegramChatId) {
      try {
        const emoji = this.getEmoji(type);
        const telegramMessage = `${emoji} *${title}*\n\n${message}`;
        await this.telegramBot.sendMessage(config.telegramChatId, telegramMessage, {
          parse_mode: 'Markdown',
          disable_web_page_preview: true
        });
      } catch (e) {
        logger.error('ALERTS', 'Telegram send failed', e.message);
      }
    }
    
    return alert;
  }
  
  getEmoji(type) {
    const emojis = {
      'rug': '🚨',
      'whale_buy': '🐋📈',
      'whale_sell': '🐋📉',
      'new_token': '🆕',
      'suspicious': '⚠️',
      'success': '✅',
      'error': '❌'
    };
    return emojis[type] || '📢';
  }
  
  getRecentAlerts(limit = 50) {
    return this.alertHistory.slice(-limit).reverse();
  }
  
  // Specific alert methods
  async alertNewToken(token) {
    const liquidity = token.initialLiquidity ? `${token.initialLiquidity.toFixed(2)} SOL` : 'Unknown';
    return this.sendAlert('new_token', 'New Token Detected', 
      `Token: ${token.name} (${token.symbol})\nMint: \`${token.mint}\`\nCreator: \`${token.creator}\`\nLiquidity: ${liquidity}`,
      token
    );
  }
  
  async alertRugPull(token, reason, severity) {
    return this.sendAlert('rug', `RUG PULL DETECTED - ${severity.toUpperCase()}`,
      `Token: ${token.symbol}\nMint: \`${token.mint}\`\nReason: ${reason}`,
      { token, reason, severity }
    );
  }
  
  async alertWhale(type, wallet, token, amountSol, amountTokens) {
    const action = type === 'buy' ? 'ACCUMULATING' : 'DUMPING';
    return this.sendAlert(`whale_${type}`, `Whale ${action}`,
      `Wallet: \`${wallet}\`\nToken: ${token.symbol}\nAmount: ${amountSol.toFixed(2)} SOL (${amountTokens.toLocaleString()} tokens)`,
      { wallet, token, amountSol, amountTokens, type }
    );
  }
  
  async alertSuspicious(token, reason) {
    return this.sendAlert('suspicious', 'Suspicious Activity',
      `Token: ${token.symbol}\nMint: \`${token.mint}\`\nReason: ${reason}`,
      { token, reason }
    );
  }
}

export const alertService = new AlertService();
export default alertService;
