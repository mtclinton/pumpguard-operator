# Build stage
FROM rust:1.75-bookworm as builder

WORKDIR /usr/src/pumpguard

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifest files
COPY Cargo.toml Cargo.lock* ./

# Create a dummy main.rs to cache dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy the actual source code
COPY src ./src
COPY public ./public

# Build the application
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary
COPY --from=builder /usr/src/pumpguard/target/release/pumpguard /app/pumpguard

# Copy public assets
COPY --from=builder /usr/src/pumpguard/public /app/public

# Create data directory
RUN mkdir -p /app/data

# Expose the dashboard port
EXPOSE 3000

# Set environment variables
ENV RUST_LOG=info,pumpguard=debug
ENV DASHBOARD_PORT=3000

# Run the application
CMD ["./pumpguard"]


