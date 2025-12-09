# 🛡️ PumpGuard

**pump.fun Monitoring Suite - Monitor Only, No Trading**

PumpGuard monitors pump.fun in real-time to detect new tokens, rug pulls, and whale movements. This is a **monitoring-only** tool - no wallet or trading functionality.

## Features

### 🆕 Token Monitor
- Real-time detection of new token launches
- Configurable filters (liquidity, creator whitelist/blacklist)
- Automatic alerts via Telegram and dashboard

### 🚨 Rug Pull Detector
- Monitors LP removal and liquidity changes
- Detects developer wallet selling patterns
- Tracks rapid/coordinated selling
- Suspicion scoring system
- Real-time alerts on suspicious activity

### 🐋 Whale Watcher
- Tracks wallets with high trading volume
- Detects accumulation and dump patterns
- Alerts on large buy/sell transactions
- Historical wallet activity tracking

### Dashboard
- Real-time WebSocket updates
- Live alert feed
- Module control toggles
- Statistics and metrics
- Prometheus metrics endpoint

## Quick Start

```bash
cd pumpguard-rs

# Configure environment
cp env.example .env
# Edit .env with your RPC URL, Telegram credentials, etc.

# Build and run
cargo run --release
```

The dashboard will be available at `http://localhost:3000`

## Configuration

Edit `pumpguard-rs/.env` with your settings:

```env
# Solana RPC (read-only)
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com
SOLANA_WS_URL=wss://api.mainnet-beta.solana.com

# Telegram Alerts (optional)
TELEGRAM_BOT_TOKEN=your_bot_token
TELEGRAM_CHAT_ID=your_chat_id

# Whale Thresholds
WHALE_THRESHOLD_SOL=50

# Rug Detection
LP_REMOVAL_THRESHOLD_PERCENT=50
SUSPICIOUS_SELL_PERCENT=10

# Dashboard
DASHBOARD_PORT=3000

# Logging level
RUST_LOG=info,pumpguard=debug
```

## Project Structure

```
pumpguard-operator/
├── pumpguard-rs/               # 🦀 Rust implementation
│   ├── Cargo.toml
│   ├── Dockerfile
│   ├── src/
│   │   ├── main.rs             # Entry point
│   │   ├── config.rs           # Configuration
│   │   ├── modules/
│   │   │   ├── token_monitor.rs
│   │   │   ├── rug_detector.rs
│   │   │   └── whale_watcher.rs
│   │   ├── dashboard/
│   │   │   └── server.rs       # Axum HTTP + WebSocket
│   │   └── utils/
│   │       ├── alerts.rs       # Telegram + WebSocket alerts
│   │       ├── database.rs     # SQLite (rusqlite)
│   │       ├── metrics.rs      # Prometheus metrics
│   │       └── solana.rs       # Solana RPC client
│   └── public/
│       └── index.html          # Dashboard UI
│
├── k8s/                        # ☸️ Kubernetes manifests
│   ├── crds/
│   ├── deploy/
│   ├── examples/
│   └── monitoring/
│
└── data/                       # SQLite database (auto-created)
```

## API Endpoints

### Stats
- `GET /api/stats` - Get all module statistics

### Token Monitor
- `GET /api/tokens/recent` - Get recently detected tokens
- `POST /api/tokens/start` - Start token monitor
- `POST /api/tokens/stop` - Stop token monitor
- `POST /api/tokens/blacklist` - Blacklist a creator

### Rug Detector
- `GET /api/rug/watched` - Get watched tokens
- `GET /api/rug/token/:mint` - Get token details
- `POST /api/rug/start` - Start rug detector
- `POST /api/rug/stop` - Stop rug detector

### Whale Watcher
- `GET /api/whales` - Get tracked whales
- `GET /api/whales/movers` - Get top token movers
- `POST /api/whales/watch` - Watch a wallet
- `POST /api/whales/start` - Start whale watcher
- `POST /api/whales/stop` - Stop whale watcher

### Alerts
- `GET /api/alerts` - Get recent alerts

### Health & Metrics
- `GET /health` - Health check
- `GET /ready` - Readiness check
- `GET /metrics` - Prometheus metrics

## WebSocket

Connect to `ws://localhost:3000/ws` for real-time updates:

```javascript
const ws = new WebSocket('ws://localhost:3000/ws');

ws.onmessage = (event) => {
  const { type, data } = JSON.parse(event.data);
  
  switch (type) {
    case 'init':
      // Initial state with stats and recent alerts
      break;
    case 'alert':
      // New alert received
      break;
  }
};
```

## Telegram Alerts

1. Create a bot with [@BotFather](https://t.me/BotFather)
2. Get your chat ID from [@userinfobot](https://t.me/userinfobot)
3. Add both to your `.env`

You'll get instant alerts for:
- 🆕 New token launches
- 🚨 Rug pull detections
- 🐋 Whale movements
- ⚠️ Suspicious activity

## Docker

```bash
cd pumpguard-rs
docker build -t pumpguard/pumpguard:latest .
docker run -p 3000:3000 --env-file .env pumpguard/pumpguard:latest
```

## Kubernetes

PumpGuard includes Kubernetes manifests for production deployments.

### Deploy

```bash
./k8s/deploy/install.sh
```

### Example PumpGuard Instance

```yaml
apiVersion: pumpguard.fun/v1
kind: PumpGuard
metadata:
  name: my-pumpguard
spec:
  rpcUrl: "https://api.mainnet-beta.solana.com"
  
  modules:
    tokenMonitor:
      enabled: true
    rugDetector:
      enabled: true
    whaleWatcher:
      enabled: true
      whaleThresholdSol: 50
  
  alerting:
    telegram:
      enabled: true
      secretRef:
        name: pumpguard-telegram
  
  dashboard:
    enabled: true
    ingress:
      enabled: true
      host: pumpguard.example.com
```

### Manage with kubectl

```bash
# List PumpGuard instances
kubectl get pumpguards

# View details
kubectl describe pumpguard my-pumpguard
```

## Prometheus Metrics

PumpGuard exposes Prometheus metrics at `/metrics` endpoint.

### Available Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `pumpguard_tokens_detected_total` | Counter | Total tokens detected |
| `pumpguard_rugs_detected_total` | Counter | Rugs detected (by severity) |
| `pumpguard_token_suspicion_score` | Gauge | Per-token suspicion score |
| `pumpguard_whales_tracked` | Gauge | Number of tracked whales |
| `pumpguard_whale_volume_sol_total` | Counter | Whale volume (by type) |
| `pumpguard_module_running` | Gauge | Module status (1=running) |
| `pumpguard_rpc_latency_seconds` | Histogram | RPC request latency |
| `pumpguard_uptime_seconds` | Gauge | Application uptime |

### Prometheus Operator Integration

```bash
kubectl apply -f k8s/monitoring/servicemonitor.yaml
```

### Grafana Dashboard

Import the pre-built dashboard from `k8s/monitoring/grafana-dashboard.json`

## RPC Recommendations

For best performance, use a premium RPC provider:
- [Helius](https://helius.xyz)
- [QuickNode](https://quicknode.com)
- [Triton](https://triton.one)

The default public RPC has rate limits that may affect performance.

## License

MIT

## Disclaimer

This software is for educational and informational purposes only. It monitors publicly available blockchain data. Use at your own risk.
