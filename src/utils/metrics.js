import { Registry, Counter, Gauge, Histogram } from 'prom-client';
import logger from './logger.js';

class MetricsService {
  constructor() {
    this.registry = new Registry();
    
    // Set default labels
    this.registry.setDefaultLabels({
      app: 'pumpguard'
    });
    
    // Initialize metrics
    this.initializeMetrics();
    
    logger.info('METRICS', 'Prometheus metrics initialized');
  }
  
  initializeMetrics() {
    // ============================================
    // TOKEN MONITOR METRICS
    // ============================================
    
    this.tokensDetected = new Counter({
      name: 'pumpguard_tokens_detected_total',
      help: 'Total number of new tokens detected',
      registers: [this.registry]
    });
    
    this.tokenAlerts = new Counter({
      name: 'pumpguard_token_alerts_total',
      help: 'Total token alerts sent',
      registers: [this.registry]
    });
    
    this.pendingTokens = new Gauge({
      name: 'pumpguard_tokens_tracked',
      help: 'Number of tokens currently being tracked',
      registers: [this.registry]
    });
    
    // ============================================
    // RUG DETECTOR METRICS
    // ============================================
    
    this.tokensWatched = new Gauge({
      name: 'pumpguard_tokens_watched',
      help: 'Number of tokens being watched for rugs',
      registers: [this.registry]
    });
    
    this.rugsDetected = new Counter({
      name: 'pumpguard_rugs_detected_total',
      help: 'Total number of rug pulls detected',
      labelNames: ['severity'],
      registers: [this.registry]
    });
    
    this.suspiciousActivity = new Counter({
      name: 'pumpguard_suspicious_activity_total',
      help: 'Total suspicious activities detected',
      labelNames: ['type'],
      registers: [this.registry]
    });
    
    this.suspicionScore = new Gauge({
      name: 'pumpguard_token_suspicion_score',
      help: 'Current suspicion score per token',
      labelNames: ['mint', 'symbol'],
      registers: [this.registry]
    });
    
    // ============================================
    // WHALE WATCHER METRICS
    // ============================================
    
    this.whalesTracked = new Gauge({
      name: 'pumpguard_whales_tracked',
      help: 'Number of whale wallets being tracked',
      registers: [this.registry]
    });
    
    this.whaleTransactions = new Counter({
      name: 'pumpguard_whale_transactions_total',
      help: 'Total whale transactions detected',
      labelNames: ['type'],
      registers: [this.registry]
    });
    
    this.whaleVolume = new Counter({
      name: 'pumpguard_whale_volume_sol_total',
      help: 'Total SOL volume from whale transactions',
      labelNames: ['type'],
      registers: [this.registry]
    });
    
    this.accumulationAlerts = new Counter({
      name: 'pumpguard_accumulation_alerts_total',
      help: 'Total accumulation pattern alerts',
      registers: [this.registry]
    });
    
    this.dumpAlerts = new Counter({
      name: 'pumpguard_dump_alerts_total',
      help: 'Total dump pattern alerts',
      registers: [this.registry]
    });
    
    // ============================================
    // SYSTEM METRICS
    // ============================================
    
    this.alertsSent = new Counter({
      name: 'pumpguard_alerts_sent_total',
      help: 'Total alerts sent',
      labelNames: ['type', 'channel'],
      registers: [this.registry]
    });
    
    this.rpcRequests = new Counter({
      name: 'pumpguard_rpc_requests_total',
      help: 'Total RPC requests made',
      labelNames: ['method', 'status'],
      registers: [this.registry]
    });
    
    this.rpcLatency = new Histogram({
      name: 'pumpguard_rpc_latency_seconds',
      help: 'RPC request latency in seconds',
      labelNames: ['method'],
      buckets: [0.01, 0.05, 0.1, 0.25, 0.5, 1, 2.5],
      registers: [this.registry]
    });
    
    this.websocketConnected = new Gauge({
      name: 'pumpguard_websocket_connected',
      help: 'WebSocket connection status (1 = connected, 0 = disconnected)',
      registers: [this.registry]
    });
    
    this.moduleStatus = new Gauge({
      name: 'pumpguard_module_running',
      help: 'Module running status (1 = running, 0 = stopped)',
      labelNames: ['module'],
      registers: [this.registry]
    });
    
    this.uptime = new Gauge({
      name: 'pumpguard_uptime_seconds',
      help: 'Application uptime in seconds',
      registers: [this.registry]
    });
    
    // Track uptime
    this.startTime = Date.now();
    setInterval(() => {
      this.uptime.set((Date.now() - this.startTime) / 1000);
    }, 10000);
  }
  
  // ============================================
  // TOKEN MONITOR METHODS
  // ============================================
  
  recordTokenDetected() {
    this.tokensDetected.inc();
  }
  
  recordTokenAlert() {
    this.tokenAlerts.inc();
  }
  
  setPendingTokens(count) {
    this.pendingTokens.set(count);
  }
  
  // ============================================
  // RUG DETECTOR METHODS
  // ============================================
  
  setTokensWatched(count) {
    this.tokensWatched.set(count);
  }
  
  recordRugDetected(severity) {
    this.rugsDetected.inc({ severity });
  }
  
  recordSuspiciousActivity(type) {
    this.suspiciousActivity.inc({ type });
  }
  
  setSuspicionScore(mint, symbol, score) {
    this.suspicionScore.set({ mint, symbol }, score);
  }
  
  // ============================================
  // WHALE WATCHER METHODS
  // ============================================
  
  setWhalesTracked(count) {
    this.whalesTracked.set(count);
  }
  
  recordWhaleTransaction(type, volumeSol) {
    this.whaleTransactions.inc({ type });
    this.whaleVolume.inc({ type }, volumeSol);
  }
  
  recordAccumulationAlert() {
    this.accumulationAlerts.inc();
  }
  
  recordDumpAlert() {
    this.dumpAlerts.inc();
  }
  
  // ============================================
  // SYSTEM METHODS
  // ============================================
  
  recordAlert(type, channel) {
    this.alertsSent.inc({ type, channel });
  }
  
  recordRpcRequest(method, status, latencyMs) {
    this.rpcRequests.inc({ method, status });
    this.rpcLatency.observe({ method }, latencyMs / 1000);
  }
  
  setWebsocketConnected(connected) {
    this.websocketConnected.set(connected ? 1 : 0);
  }
  
  setModuleStatus(module, running) {
    this.moduleStatus.set({ module }, running ? 1 : 0);
  }
  
  // Get metrics for Prometheus scraping
  async getMetrics() {
    return this.registry.metrics();
  }
  
  getContentType() {
    return this.registry.contentType;
  }
}

export const metricsService = new MetricsService();
export default metricsService;
