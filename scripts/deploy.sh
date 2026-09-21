#!/bin/bash
set -e

# ============================================
# Deploy script for HISENSE LingxiBI
# Usage: ./scripts/deploy.sh user@host [domain]
#
# Architecture:
#   - Rust/axum API server  -> systemd service "ai-report", listens on 127.0.0.1:3001
#   - Vite React SPA        -> static files served by Nginx on port 9528
#   - Nginx                 -> serves SPA (:9528) + proxies /api (incl. /api/chat WS) -> 3001
#   - MySQL                 -> metadata DB (migrations auto-run on startup)
#
# The Rust binary is built ON THE SERVER to avoid glibc/arch mismatch.
# Run scripts/setup-server.sh once on a fresh server before the first deploy.
# ============================================

SERVER=${1:?"Usage: ./scripts/deploy.sh user@host [domain]"}
DOMAIN=${2:-}
APP_DIR="/opt/ai-report"
PORT=3001

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

echo ""
echo "  ┌─────────────────────────────┐"
echo "  │  HISENSE LingxiBI Deploy            │"
echo "  └─────────────────────────────┘"
echo ""

# Step 1: Build the client (portable static assets) locally
echo "[1/6] Building client..."
( cd client && npm install --silent && npm run build )
[ -d client/dist ] || { echo "ERROR: client/dist not found after build"; exit 1; }
echo "      Client build complete"

# Step 2: Package client dist + server source (binary is built on the server)
echo "[2/6] Packaging..."
tar -czf /tmp/ai-report-deploy.tar.gz \
  client/dist \
  server/src \
  server/migrations \
  server/Cargo.toml \
  server/Cargo.lock
FILESIZE=$(du -h /tmp/ai-report-deploy.tar.gz | cut -f1)
echo "      Package size: $FILESIZE"

# Step 3: Upload
echo "[3/6] Uploading to $SERVER..."
scp -q /tmp/ai-report-deploy.tar.gz "$SERVER:/tmp/ai-report-deploy.tar.gz"
rm -f /tmp/ai-report-deploy.tar.gz
echo "      Upload complete"

# Step 4 + 5 + 6 happen on the server
echo "[4/6] Building & deploying on server..."
ssh "$SERVER" APP_DIR="$APP_DIR" PORT="$PORT" bash -s <<'REMOTE'
set -e

APP_DIR=${APP_DIR:-/opt/ai-report}
PORT=${PORT:-3001}

# --- Toolchain ---
export PATH="$PATH:/usr/local/bin:/usr/bin:$HOME/.cargo/bin"
if ! command -v cargo > /dev/null 2>&1; then
  [ -s "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
fi
command -v cargo > /dev/null 2>&1 || { echo "ERROR: cargo not found. Run scripts/setup-server.sh first."; exit 1; }
echo "      Rust: $(cargo --version)"

# --- Extract (preserve existing .env across deploys) ---
echo "      Extracting..."
mkdir -p "$APP_DIR"
tar -xzf /tmp/ai-report-deploy.tar.gz -C "$APP_DIR"
rm -f /tmp/ai-report-deploy.tar.gz

# --- Server .env: seed on first deploy, never overwrite afterwards ---
ENV_FILE="$APP_DIR/server/.env"
if [ ! -f "$ENV_FILE" ]; then
  echo "      Seeding default .env (EDIT THIS with real values!)..."
  JWT=$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
  cat > "$ENV_FILE" <<EOF
DATABASE_URL=mysql://root:changeme@localhost:3306/ai_report
JWT_SECRET=$JWT
CORS_ALLOWED_ORIGIN=*
EOF
  echo "      WARNING: edit $ENV_FILE and set the real DATABASE_URL, then re-run deploy."
fi

# --- Build release binary on the server ---
echo "      Building Rust server (release)..."
( cd "$APP_DIR/server" && cargo build --release 2>&1 | tail -5 )
BIN="$APP_DIR/server/target/release/ai-report-server"
[ -x "$BIN" ] || { echo "ERROR: build failed, binary not found"; exit 1; }
echo "      Binary ready: $($BIN --version 2>/dev/null || echo built)"

# --- systemd service ---
echo "      Installing systemd service..."
sudo tee /etc/systemd/system/ai-report.service > /dev/null <<EOF
[Unit]
Description=HISENSE LingxiBI API server
After=network.target mysql.service mysqld.service mariadb.service

[Service]
Type=simple
WorkingDirectory=$APP_DIR/server
ExecStart=$BIN
EnvironmentFile=$APP_DIR/server/.env
Restart=on-failure
RestartSec=3
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable ai-report > /dev/null 2>&1 || true

echo "      Restarting service..."
sudo systemctl restart ai-report
sleep 4

# --- Health check ---
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:$PORT/api/health 2>/dev/null || echo "000")
if systemctl is-active --quiet ai-report && [ "$HTTP_CODE" = "200" ]; then
  echo "      Health check passed (HTTP $HTTP_CODE)"
else
  echo "      WARNING: service active=$(systemctl is-active ai-report) HTTP=$HTTP_CODE"
  sudo journalctl -u ai-report -n 20 --no-pager 2>/dev/null || true
fi

# --- Reload nginx if present ---
if command -v nginx > /dev/null 2>&1; then
  sudo nginx -t 2>/dev/null && sudo systemctl reload nginx 2>/dev/null || true
fi

echo ""
echo "      Deploy successful!"
REMOTE

echo ""
if [ -n "$DOMAIN" ]; then
  echo "  Done. Site: https://$DOMAIN:9528"
else
  echo "  Done. SPA served on port 9528 (run scripts/setup-server.sh first if Nginx isn't configured)."
fi
echo ""
