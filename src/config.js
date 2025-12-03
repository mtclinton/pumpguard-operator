import dotenv from 'dotenv';
dotenv.config();

export const config = {
  // Solana RPC (read-only, no wallet needed)
  rpcUrl: process.env.SOLANA_RPC_URL || 'https://api.mainnet-beta.solana.com',
  wsUrl: process.env.SOLANA_WS_URL || 'wss://api.mainnet-beta.solana.com',
  
  // Pump.fun
  pumpProgramId: process.env.PUMP_PROGRAM_ID || '6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P',
  
  // Telegram Alerts
  telegramBotToken: process.env.TELEGRAM_BOT_TOKEN,
  telegramChatId: process.env.TELEGRAM_CHAT_ID,
  
  // Whale Watcher
  whaleThresholdSol: parseFloat(process.env.WHALE_THRESHOLD_SOL) || 50,
  alertOnAccumulation: process.env.ALERT_ON_ACCUMULATION !== 'false',
  alertOnDump: process.env.ALERT_ON_DUMP !== 'false',
  
  // Rug Detection
  lpRemovalThresholdPercent: parseFloat(process.env.LP_REMOVAL_THRESHOLD_PERCENT) || 50,
  suspiciousSellPercent: parseFloat(process.env.SUSPICIOUS_SELL_PERCENT) || 10,
  devWalletSellAlert: process.env.DEV_WALLET_SELL_ALERT !== 'false',
  
  // Dashboard
  dashboardPort: parseInt(process.env.DASHBOARD_PORT) || 3000,
};

export default config;
