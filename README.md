# 🛡️ PumpGuard

![PumpGuard](pumpguard.png)

**pump.fun Monitoring Suite - Monitor Only, No Trading**

PumpGuard monitors pump.fun in real-time to detect new tokens, rug pulls, and whale movements. This is a **monitoring-only** tool - no wallet or trading functionality.

## Features

- 🆕 **Token Monitor** - Real-time detection of new token launches
- 🚨 **Rug Pull Detector** - Monitors LP removal and suspicious activity
- 🐋 **Whale Watcher** - Tracks large wallet movements
- 📊 **Dashboard** - WebSocket-powered real-time dashboard

## How to Run

### Prerequisites

- Rust 1.70+ (with Cargo)
- OpenSSL development libraries

### Quick Start

```bash
cd pumpguard-rs

# Configure environment (if not already done)
cp env.example .env
# Edit .env with your RPC URL, Telegram credentials, etc.

# Build and run
cargo run --release
```

The dashboard will be available at `http://localhost:3000`

### Run with Docker

```bash
cd pumpguard-rs
docker build -t pumpguard/pumpguard:latest .
docker run -p 3000:3000 --env-file .env pumpguard/pumpguard:latest
```

### Build Binary

```bash
cd pumpguard-rs
cargo build --release
./target/release/pumpguard
```

## Configuration

Edit `pumpguard-rs/.env` with your settings. At minimum, configure:

```env
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com
SOLANA_WS_URL=wss://api.mainnet-beta.solana.com
DASHBOARD_PORT=3000
```

Telegram alerts are optional but recommended for real-time notifications.

## License

MIT

## Disclaimer

This software is for educational and informational purposes only. It monitors publicly available blockchain data. Use at your own risk.
