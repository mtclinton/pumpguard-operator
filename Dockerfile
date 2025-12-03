# PumpGuard Docker Image
FROM node:20-alpine

WORKDIR /app

# Install dependencies
COPY package*.json ./
RUN npm ci --only=production

# Copy source code
COPY src/ ./src/
COPY data/ ./data/

# Create non-root user
RUN addgroup -g 1001 -S pumpguard && \
    adduser -S pumpguard -u 1001 -G pumpguard && \
    chown -R pumpguard:pumpguard /app

USER pumpguard

# Expose dashboard port
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
  CMD wget --no-verbose --tries=1 --spider http://localhost:3000/api/stats || exit 1

CMD ["node", "src/index.js"]

