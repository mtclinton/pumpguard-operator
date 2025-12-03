import config from './config.js';
import logger from './utils/logger.js';
import tokenMonitor from './modules/tokenMonitor.js';
import rugDetector from './modules/rugDetector.js';
import whaleWatcher from './modules/whaleWatcher.js';
import dashboard from './dashboard/server.js';

class PumpGuard {
  constructor() {
    this.modules = {
      tokenMonitor,
      rugDetector,
      whaleWatcher
    };
  }
  
  async start() {
    console.log(`
    ╔═══════════════════════════════════════════════════════════════╗
    ║                                                               ║
    ║   ██████╗ ██╗   ██╗███╗   ███╗██████╗  ██████╗ ██╗   ██╗ █████╗ ██████╗ ██████╗  ║
    ║   ██╔══██╗██║   ██║████╗ ████║██╔══██╗██╔════╝ ██║   ██║██╔══██╗██╔══██╗██╔══██╗ ║
    ║   ██████╔╝██║   ██║██╔████╔██║██████╔╝██║  ███╗██║   ██║███████║██████╔╝██║  ██║ ║
    ║   ██╔═══╝ ██║   ██║██║╚██╔╝██║██╔═══╝ ██║   ██║██║   ██║██╔══██║██╔══██╗██║  ██║ ║
    ║   ██║     ╚██████╔╝██║ ╚═╝ ██║██║     ╚██████╔╝╚██████╔╝██║  ██║██║  ██║██████╔╝ ║
    ║   ╚═╝      ╚═════╝ ╚═╝     ╚═╝╚═╝      ╚═════╝  ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝╚═════╝  ║
    ║                                                               ║
    ║   🛡️  pump.fun Monitoring Suite (Monitor-Only Mode)           ║
    ║   🆕 New Tokens | 🚨 Rug Detector | 🐋 Whale Watcher          ║
    ║                                                               ║
    ╚═══════════════════════════════════════════════════════════════╝
    `);
    
    logger.info('PUMPGUARD', 'Initializing PumpGuard Monitor...');
    
    // Start dashboard
    dashboard.start();
    
    // Start all modules
    logger.info('PUMPGUARD', 'Starting monitoring modules...');
    
    await Promise.all([
      tokenMonitor.start(),
      rugDetector.start(),
      whaleWatcher.start()
    ]);
    
    // Link modules: auto-watch new tokens for rug detection
    this.linkModules();
    
    logger.success('PUMPGUARD', 'All modules started successfully!');
    logger.info('PUMPGUARD', `Dashboard: http://localhost:${config.dashboardPort}`);
    
    // Setup graceful shutdown
    this.setupShutdown();
  }
  
  linkModules() {
    // When token monitor detects a new token, add it to rug detector watch list
    const originalHandleNewToken = tokenMonitor.handleNewToken.bind(tokenMonitor);
    tokenMonitor.handleNewToken = async (signature) => {
      const tokenInfo = await originalHandleNewToken(signature);
      
      if (tokenInfo && !rugDetector.watchedTokens.has(tokenInfo.mint)) {
        rugDetector.watchToken(tokenInfo);
      }
      
      return tokenInfo;
    };
    
    logger.info('PUMPGUARD', 'Modules linked - new tokens auto-watched by rug detector');
  }
  
  setupShutdown() {
    const shutdown = async () => {
      logger.info('PUMPGUARD', 'Shutting down...');
      
      await Promise.all([
        tokenMonitor.stop(),
        rugDetector.stop(),
        whaleWatcher.stop()
      ]);
      
      logger.success('PUMPGUARD', 'Shutdown complete');
      process.exit(0);
    };
    
    process.on('SIGINT', shutdown);
    process.on('SIGTERM', shutdown);
  }
  
  getStatus() {
    return {
      tokenMonitor: tokenMonitor.getStats(),
      rugDetector: rugDetector.getStats(),
      whaleWatcher: whaleWatcher.getStats()
    };
  }
}

// Start the application
const pumpGuard = new PumpGuard();
pumpGuard.start().catch(error => {
  logger.error('PUMPGUARD', 'Fatal error', error.message);
  process.exit(1);
});

export default pumpGuard;
