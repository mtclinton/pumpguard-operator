import express from 'express';
import { WebSocketServer } from 'ws';
import { createServer } from 'http';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';
import config from '../config.js';
import logger from '../utils/logger.js';
import alertService from '../utils/alerts.js';
import databaseService from '../utils/database.js';
import metricsService from '../utils/metrics.js';
import tokenMonitor from '../modules/tokenMonitor.js';
import rugDetector from '../modules/rugDetector.js';
import whaleWatcher from '../modules/whaleWatcher.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

class Dashboard {
  constructor() {
    this.app = express();
    this.server = createServer(this.app);
    this.wss = new WebSocketServer({ server: this.server });
    this.clients = new Set();
    
    this.setupMiddleware();
    this.setupRoutes();
    this.setupWebSocket();
  }
  
  setupMiddleware() {
    this.app.use(express.json());
    this.app.use(express.static(join(__dirname, 'public')));
  }
  
  setupRoutes() {
    // API Routes
    
    // Stats
    this.app.get('/api/stats', (req, res) => {
      res.json({
        tokenMonitor: tokenMonitor.getStats(),
        rugDetector: rugDetector.getStats(),
        whaleWatcher: whaleWatcher.getStats()
      });
    });
    
    // Token Monitor
    this.app.get('/api/tokens/recent', (req, res) => {
      const limit = parseInt(req.query.limit) || 50;
      res.json(tokenMonitor.getRecentTokens(limit));
    });
    
    this.app.post('/api/tokens/start', async (req, res) => {
      await tokenMonitor.start();
      res.json({ success: true, message: 'Token monitor started' });
    });
    
    this.app.post('/api/tokens/stop', async (req, res) => {
      await tokenMonitor.stop();
      res.json({ success: true, message: 'Token monitor stopped' });
    });
    
    this.app.post('/api/tokens/filter', (req, res) => {
      const { key, value } = req.body;
      tokenMonitor.setFilter(key, value);
      res.json({ success: true });
    });
    
    this.app.post('/api/tokens/blacklist', (req, res) => {
      const { address } = req.body;
      tokenMonitor.blacklistCreator(address);
      res.json({ success: true });
    });
    
    // Rug Detector
    this.app.get('/api/rug/watched', (req, res) => {
      res.json(rugDetector.getWatchedTokens());
    });
    
    this.app.get('/api/rug/token/:mint', (req, res) => {
      const details = rugDetector.getTokenDetails(req.params.mint);
      res.json(details || { error: 'Token not found' });
    });
    
    this.app.post('/api/rug/watch', (req, res) => {
      const { token } = req.body;
      rugDetector.watchToken(token);
      res.json({ success: true });
    });
    
    this.app.post('/api/rug/start', async (req, res) => {
      await rugDetector.start();
      res.json({ success: true, message: 'Rug detector started' });
    });
    
    this.app.post('/api/rug/stop', async (req, res) => {
      await rugDetector.stop();
      res.json({ success: true, message: 'Rug detector stopped' });
    });
    
    // Whale Watcher
    this.app.get('/api/whales', (req, res) => {
      res.json(whaleWatcher.getWhales());
    });
    
    this.app.get('/api/whales/movers', (req, res) => {
      res.json(whaleWatcher.getTopMovers());
    });
    
    this.app.get('/api/whales/wallet/:address', (req, res) => {
      const activity = whaleWatcher.getWalletActivity(req.params.address);
      res.json(activity || { error: 'Wallet not found' });
    });
    
    this.app.post('/api/whales/watch', (req, res) => {
      const { address, label } = req.body;
      whaleWatcher.watchWallet(address, label);
      res.json({ success: true });
    });
    
    this.app.post('/api/whales/start', async (req, res) => {
      await whaleWatcher.start();
      res.json({ success: true, message: 'Whale watcher started' });
    });
    
    this.app.post('/api/whales/stop', async (req, res) => {
      await whaleWatcher.stop();
      res.json({ success: true, message: 'Whale watcher stopped' });
    });
    
    // Alerts
    this.app.get('/api/alerts', (req, res) => {
      const limit = parseInt(req.query.limit) || 50;
      res.json(alertService.getRecentAlerts(limit));
    });
    
    // Tokens from database
    this.app.get('/api/tokens', (req, res) => {
      const limit = parseInt(req.query.limit) || 50;
      res.json(databaseService.getRecentTokens(limit));
    });
    
    this.app.get('/api/tokens/:mint', (req, res) => {
      const token = databaseService.getToken(req.params.mint);
      res.json(token || { error: 'Token not found' });
    });
    
    // Prometheus metrics endpoint
    this.app.get('/metrics', async (req, res) => {
      try {
        // Update current state metrics
        metricsService.setPendingTokens(tokenMonitor.detectedTokens.size);
        metricsService.setTokensWatched(rugDetector.watchedTokens.size);
        metricsService.setWhalesTracked(whaleWatcher.watchedWallets.size);
        metricsService.setModuleStatus('tokenMonitor', tokenMonitor.isRunning);
        metricsService.setModuleStatus('rugDetector', rugDetector.isRunning);
        metricsService.setModuleStatus('whaleWatcher', whaleWatcher.isRunning);
        
        res.set('Content-Type', metricsService.getContentType());
        res.end(await metricsService.getMetrics());
      } catch (e) {
        res.status(500).end(e.message);
      }
    });
    
    // Health check endpoint
    this.app.get('/health', (req, res) => {
      res.json({
        status: 'healthy',
        uptime: process.uptime(),
        modules: {
          tokenMonitor: tokenMonitor.isRunning,
          rugDetector: rugDetector.isRunning,
          whaleWatcher: whaleWatcher.isRunning
        }
      });
    });
    
    // Readiness check
    this.app.get('/ready', (req, res) => {
      const ready = tokenMonitor.isRunning || rugDetector.isRunning || whaleWatcher.isRunning;
      res.status(ready ? 200 : 503).json({ ready });
    });
    
    // Serve dashboard
    this.app.get('/', (req, res) => {
      res.sendFile(join(__dirname, 'public', 'index.html'));
    });
  }
  
  setupWebSocket() {
    this.wss.on('connection', (ws) => {
      this.clients.add(ws);
      logger.info('DASHBOARD', 'Client connected');
      
      // Send initial state
      ws.send(JSON.stringify({
        type: 'init',
        data: {
          stats: {
            tokenMonitor: tokenMonitor.getStats(),
            rugDetector: rugDetector.getStats(),
            whaleWatcher: whaleWatcher.getStats()
          },
          recentAlerts: alertService.getRecentAlerts(20)
        }
      }));
      
      ws.on('close', () => {
        this.clients.delete(ws);
        logger.info('DASHBOARD', 'Client disconnected');
      });
      
      ws.on('error', (error) => {
        logger.error('DASHBOARD', 'WebSocket error', error.message);
        this.clients.delete(ws);
      });
    });
    
    // Subscribe to alerts
    alertService.subscribe((alert) => {
      this.broadcast({
        type: 'alert',
        data: alert
      });
    });
  }
  
  broadcast(message) {
    const data = JSON.stringify(message);
    for (const client of this.clients) {
      if (client.readyState === 1) { // WebSocket.OPEN
        client.send(data);
      }
    }
  }
  
  start() {
    const port = config.dashboardPort;
    this.server.listen(port, () => {
      logger.success('DASHBOARD', `Dashboard running at http://localhost:${port}`);
    });
  }
}

export const dashboard = new Dashboard();
export default dashboard;
