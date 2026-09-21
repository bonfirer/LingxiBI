#!/bin/bash
set -e

# ============================================
# One-time server provisioning for HISENSE LingxiBI
# Run ON the server (as root or with sudo):
#   bash setup-server.sh [domain] [email]
#
# Installs: Rust toolchain, build deps, MySQL, Nginx (+ optional Let's Encrypt SSL)
# Configures Nginx to serve the SPA from /opt/ai-report/client/dist and
# proxy /api (incl. the /api/chat WebSocket) to 127.0.0.1:3001.
#
# After this, run scripts/deploy.sh from your machine.
# ============================================

DOMAIN=${1:-}
EMAIL=${2:-}
APP_DIR="/opt/ai-report"
APP_PORT=3001          # internal Rust API server
FRONTEND_PORT=9528     # public port Nginx serves the SPA on
DIST_DIR="$APP_DIR/client/dist"

SUDO=""
[ "$(id -u)" -ne 0 ] && SUDO="sudo"

# --- Detect package manager ---
if command -v apt-get > /dev/null 2>&1; then
  PM="apt"
elif command -v dnf > /dev/null 2>&1; then
  PM="dnf"
elif command -v yum > /dev/null 2>&1; then
  PM="yum"
else
  echo "ERROR: no supported package manager (apt/dnf/yum) found"
  exit 1
fi
echo "=> Package manager: $PM"

# --- Install build deps, nginx, mysql, certbot ---
echo "=> Installing system packages..."
case "$PM" in
  apt)
    $SUDO apt-get update -y
    $SUDO apt-get install -y curl build-essential pkg-config libssl-dev \
      nginx default-mysql-server certbot python3-certbot-nginx
    ;;
  dnf|yum)
    $SUDO $PM install -y epel-release 2>/dev/null || true
    $SUDO $PM groupinstall -y "Development Tools" || \
      $SUDO $PM install -y gcc gcc-c++ make
    $SUDO $PM install -y curl pkgconfig openssl-devel nginx mariadb-server
    # certbot is optional (SSL is skipped on custom ports); install if available
    $SUDO $PM install -y certbot python3-certbot-nginx 2>/dev/null || \
      echo "   (certbot not available in repos — skipping, SSL is handled separately)"
    ;;
esac

# --- Rust toolchain (rustup) ---
if ! command -v cargo > /dev/null 2>&1; then
  echo "=> Installing Rust toolchain..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  . "$HOME/.cargo/env"
fi
echo "=> Rust: $(cargo --version)"

# --- Start MySQL/MariaDB ---
echo "=> Enabling database service..."
for svc in mysqld mariadb mysql; do
  if systemctl list-unit-files | grep -q "^$svc"; then
    $SUDO systemctl enable "$svc" 2>/dev/null || true
    $SUDO systemctl start "$svc" 2>/dev/null || true
    break
  fi
done
echo "   NOTE: create the database and a user, e.g.:"
echo "     CREATE DATABASE ai_report CHARACTER SET utf8mb4;"
echo "   then set DATABASE_URL in $APP_DIR/server/.env"

# --- TLS certificate ---
# Preference order:
#   1. An existing Let's Encrypt cert whose SAN covers $DOMAIN (browser-trusted)
#   2. A self-signed cert (works for bare IPs; browsers warn)
SSL_DIR="/etc/nginx/ssl"
SSL_CRT="$SSL_DIR/ai-report.crt"
SSL_KEY="$SSL_DIR/ai-report.key"

if [ -n "$DOMAIN" ]; then
  for d in /etc/letsencrypt/live/*/; do
    [ -f "$d/fullchain.pem" ] || continue
    if openssl x509 -in "$d/fullchain.pem" -noout -ext subjectAltName 2>/dev/null | grep -qw "$DOMAIN"; then
      SSL_CRT="$d/fullchain.pem"
      SSL_KEY="$d/privkey.pem"
      echo "=> Using trusted Let's Encrypt cert for $DOMAIN ($d)."
      break
    fi
  done
fi

if [ "$SSL_CRT" = "$SSL_DIR/ai-report.crt" ]; then
  $SUDO mkdir -p "$SSL_DIR"
  if [ ! -f "$SSL_CRT" ] || [ ! -f "$SSL_KEY" ]; then
    echo "=> Generating self-signed TLS certificate..."
    CN="${DOMAIN:-$(curl -s --max-time 5 ifconfig.me 2>/dev/null || hostname -I | awk '{print $1}')}"
    if echo "$CN" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'; then
      SAN="IP:$CN"
    else
      SAN="DNS:$CN"
    fi
    $SUDO openssl req -x509 -nodes -newkey rsa:2048 \
      -keyout "$SSL_KEY" -out "$SSL_CRT" -days 3650 \
      -subj "/CN=$CN" -addext "subjectAltName=$SAN" 2>/dev/null
    echo "   Cert CN=$CN ($SAN). Browsers will warn until replaced with a trusted cert."
  fi
fi

# --- Nginx config ---
echo "=> Writing Nginx config..."
mkdir -p "$DIST_DIR"

write_https() {
  cat <<EOF
server {
    listen $FRONTEND_PORT ssl;
    http2 on;
    server_name ${DOMAIN:-_};

    ssl_certificate     $SSL_CRT;
    ssl_certificate_key $SSL_KEY;
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers HIGH:!aNULL:!MD5;
    ssl_prefer_server_ciphers on;
    ssl_session_cache shared:SSL:10m;

    add_header X-Frame-Options SAMEORIGIN;
    add_header X-Content-Type-Options nosniff;

    root $DIST_DIR;
    index index.html;

    # Redirect plain HTTP hitting this TLS port to HTTPS
    error_page 497 =301 https://\$host:$FRONTEND_PORT\$request_uri;

    # API + WebSocket -> Rust server
    location /api/ {
        proxy_pass http://127.0.0.1:$APP_PORT;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 3600s;
        client_max_body_size 12m;
    }

    # SPA fallback
    location / {
        try_files \$uri \$uri/ /index.html;
    }

    # Cache hashed static assets
    location /assets/ {
        expires 365d;
        add_header Cache-Control "public, immutable";
    }
}
EOF
}

# Pick config location based on distro layout
if [ -d /etc/nginx/conf.d ]; then
  NGINX_CONF="/etc/nginx/conf.d/ai-report.conf"
else
  NGINX_CONF="/etc/nginx/sites-available/ai-report.conf"
fi

write_https | $SUDO tee "$NGINX_CONF" > /dev/null

# Debian/Ubuntu: enable site + drop default
if [ -d /etc/nginx/sites-enabled ]; then
  $SUDO ln -sfn "$NGINX_CONF" /etc/nginx/sites-enabled/ai-report.conf
  $SUDO rm -f /etc/nginx/sites-enabled/default
fi

$SUDO nginx -t
$SUDO systemctl enable nginx 2>/dev/null || true
$SUDO systemctl restart nginx
echo "=> Nginx running (HTTPS on $FRONTEND_PORT)."

# --- Trusted SSL note ---
# This uses a self-signed cert (works for bare IPs). For a browser-trusted cert
# you need a domain: point it at this server and either
#   - run certbot for a standard 80/443 vhost, or
#   - drop a real cert into $SSL_DIR (ai-report.crt / ai-report.key) and reload nginx.
if [ -n "$DOMAIN" ]; then
  echo "=> For a trusted cert on $DOMAIN, replace $SSL_DIR/ai-report.{crt,key} and reload nginx."
fi

# --- Open the frontend port in the firewall (best effort) ---
if command -v firewall-cmd > /dev/null 2>&1; then
  $SUDO firewall-cmd --permanent --add-port=${FRONTEND_PORT}/tcp 2>/dev/null || true
  $SUDO firewall-cmd --reload 2>/dev/null || true
elif command -v ufw > /dev/null 2>&1; then
  $SUDO ufw allow ${FRONTEND_PORT}/tcp 2>/dev/null || true
fi

echo ""
echo "========================================="
echo "  Server provisioned. SPA served over HTTPS on port $FRONTEND_PORT."
echo "  1. Create the MySQL database + user."
echo "  2. Edit $APP_DIR/server/.env (DATABASE_URL, JWT_SECRET)."
echo "  3. Deploy from your machine:"
echo "       ./scripts/deploy.sh user@host ${DOMAIN}"
echo "========================================="
echo ""
