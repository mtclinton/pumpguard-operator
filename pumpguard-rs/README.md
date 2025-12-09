# 🛡️ PumpGuard (Rust Edition)

**pump.fun Monitoring Suite - Monitor Only, No Trading**

PumpGuard monitors pump.fun in real-time to detect new tokens, rug pulls, and whale movements. This is a **monitoring-only** tool - no wallet or trading functionality.

This is a complete rewrite of PumpGuard in Rust for improved performance and reliability.

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

## Building

### Prerequisites

- Rust 1.70+ (with Cargo)
- OpenSSL development libraries

### Build from Source

```bash
# Clone the repository
git clone https://github.com/yourusername/pumpguard.git
cd pumpguard/pumpguard-rs

# Build in release mode
cargo build --release

# The binary will be at target/release/pumpguard
```

### Build with Docker

```bash
docker build -t pumpguard/pumpguard-rs:latest .
```

## Configuration

Copy the example environment file and edit it:

```bash
cp env.example .env
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SOLANA_RPC_URL` | `https://api.mainnet-beta.solana.com` | Solana RPC endpoint |
| `SOLANA_WS_URL` | `wss://api.mainnet-beta.solana.com` | Solana WebSocket endpoint |
| `PUMP_PROGRAM_ID` | `6EF8rrecthR5D...` | pump.fun program ID |
| `TELEGRAM_BOT_TOKEN` | - | Telegram bot token (optional) |
| `TELEGRAM_CHAT_ID` | - | Telegram chat ID (optional) |
| `WHALE_THRESHOLD_SOL` | `50` | SOL threshold for whale detection |
| `ALERT_ON_ACCUMULATION` | `true` | Alert on whale buys |
| `ALERT_ON_DUMP` | `true` | Alert on whale sells |
| `LP_REMOVAL_THRESHOLD_PERCENT` | `50` | LP removal alert threshold |
| `SUSPICIOUS_SELL_PERCENT` | `10` | Large sell alert threshold |
| `DEV_WALLET_SELL_ALERT` | `true` | Alert on dev wallet sells |
| `DASHBOARD_PORT` | `3000` | Dashboard HTTP port |
| `RUST_LOG` | `info,pumpguard=debug` | Log level configuration |

## Usage

### Run Directly

```bash
# Make sure .env is configured
cargo run --release
```

### Run with Docker

```bash
docker run -p 3000:3000 --env-file .env pumpguard/pumpguard-rs:latest
```

### Run with Docker Compose

```yaml
version: '3.8'
services:
  pumpguard:
    build: .
    ports:
      - "3000:3000"
    environment:
      - SOLANA_RPC_URL=https://api.mainnet-beta.solana.com
      - SOLANA_WS_URL=wss://api.mainnet-beta.solana.com
      - DASHBOARD_PORT=3000
    volumes:
      - ./data:/app/data
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
  const message = JSON.parse(event.data);
  
  switch (message.type) {
    case 'init':
      // Initial state with stats and recent alerts
      console.log('Stats:', message.data.stats);
      console.log('Alerts:', message.data.recent_alerts);
      break;
    case 'alert':
      // New alert received
      console.log('New alert:', message.data);
      break;
  }
};
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

## Project Structure

```
pumpguard-rs/
├── Cargo.toml              # Dependencies and metadata
├── Dockerfile              # Container build file
├── env.example             # Example environment config
├── README.md               # This file
├── public/
│   └── index.html          # Dashboard UI
└── src/
    ├── main.rs             # Entry point
    ├── config.rs           # Configuration loader
    ├── modules/
    │   ├── mod.rs
    │   ├── token_monitor.rs    # New token detection
    │   ├── rug_detector.rs     # Rug pull detector
    │   └── whale_watcher.rs    # Whale watcher module
    ├── dashboard/
    │   ├── mod.rs
    │   └── server.rs           # HTTP + WebSocket server
    └── utils/
        ├── mod.rs
        ├── alerts.rs           # Alert service (Telegram + WS)
        ├── database.rs         # SQLite database service
        ├── logger.rs           # Logging configuration
        ├── metrics.rs          # Prometheus metrics
        └── solana.rs           # Solana RPC connection
```

## Performance

The Rust implementation provides several advantages over the Node.js version:

- **Lower memory footprint**: ~50MB vs ~150MB
- **Better CPU efficiency**: Native async runtime
- **Faster startup**: ~100ms vs ~2s
- **Single binary**: No runtime dependencies
- **Type safety**: Compile-time error checking

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


