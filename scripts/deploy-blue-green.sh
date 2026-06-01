#!/usr/bin/env bash
# Blue/green zero-downtime deployment for Trivela backend.
#
# Usage:
#   DEPLOY_IMAGE=trivela-backend:v1.2.3 ./scripts/deploy-blue-green.sh
#
# Environment variables:
#   DEPLOY_IMAGE          Docker image to deploy (required)
#   DEPLOY_STRATEGY       blue-green | rolling | recreate  (default: blue-green)
#   HEALTH_CHECK_URL      Override health endpoint (default: http://localhost:PORT/health)
#   BLUE_PORT             Port for the blue container (default: 3001)
#   GREEN_PORT            Port for the green container (default: 3002)
#   NGINX_CONF            Nginx upstream config file to rewrite (default: /etc/nginx/conf.d/trivela_upstream.conf)
#   MAX_HEALTH_WAIT       Seconds to wait for green to become healthy (default: 60)
#   SETTLE_WAIT           Seconds to watch green after cut-over before stopping blue (default: 30)

set -euo pipefail

DEPLOY_IMAGE="${DEPLOY_IMAGE:-}"
DEPLOY_STRATEGY="${DEPLOY_STRATEGY:-blue-green}"
BLUE_PORT="${BLUE_PORT:-3001}"
GREEN_PORT="${GREEN_PORT:-3002}"
NGINX_CONF="${NGINX_CONF:-/etc/nginx/conf.d/trivela_upstream.conf}"
MAX_HEALTH_WAIT="${MAX_HEALTH_WAIT:-60}"
SETTLE_WAIT="${SETTLE_WAIT:-30}"

BLUE_CONTAINER="trivela-backend-blue"
GREEN_CONTAINER="trivela-backend-green"

log()  { printf '\033[0;36m[deploy]\033[0m  %s\n' "$*"; }
ok()   { printf '\033[0;32m[  ok  ]\033[0m  %s\n' "$*"; }
warn() { printf '\033[0;33m[ warn ]\033[0m  %s\n' "$*"; }
err()  { printf '\033[0;31m[ fail ]\033[0m  %s\n' "$*" >&2; }

if [[ -z "$DEPLOY_IMAGE" ]]; then
  err "DEPLOY_IMAGE is required. Example: DEPLOY_IMAGE=trivela-backend:v1.2.3 $0"
  exit 1
fi

if [[ "$DEPLOY_STRATEGY" != "blue-green" ]]; then
  warn "DEPLOY_STRATEGY=$DEPLOY_STRATEGY — only blue-green is implemented here. Exiting."
  exit 0
fi

HEALTH_URL="${HEALTH_CHECK_URL:-http://localhost:${GREEN_PORT}/health}"

detect_active_color() {
  if docker inspect "$BLUE_CONTAINER" &>/dev/null; then
    echo "blue"
  else
    echo "none"
  fi
}

write_upstream() {
  local port="$1"
  cat >"$NGINX_CONF" <<EOF
upstream trivela_backend {
  server 127.0.0.1:${port};
}
EOF
  nginx -s reload
}

rollback() {
  err "Rollback triggered — switching upstream back to blue on port ${BLUE_PORT}"
  if [[ -f "$NGINX_CONF" ]]; then
    write_upstream "$BLUE_PORT" || warn "Nginx reload failed during rollback"
  fi
  if docker inspect "$GREEN_CONTAINER" &>/dev/null; then
    docker stop "$GREEN_CONTAINER" || true
    docker rm "$GREEN_CONTAINER" || true
    warn "Green container stopped and removed."
  fi
  err "Deployment failed. Blue is still serving traffic."
  exit 1
}

trap rollback ERR

log "Starting blue/green deployment of ${DEPLOY_IMAGE}"

ACTIVE=$(detect_active_color)
log "Current active slot: ${ACTIVE:-none}"

log "Launching green container on port ${GREEN_PORT}..."
docker run -d \
  --name "$GREEN_CONTAINER" \
  --restart unless-stopped \
  -p "${GREEN_PORT}:3001" \
  --env-file .env \
  "$DEPLOY_IMAGE"

log "Waiting for green to pass health checks (max ${MAX_HEALTH_WAIT}s)..."
WAITED=0
until curl -sf "$HEALTH_URL" | grep -q '"status"'; do
  if [[ $WAITED -ge $MAX_HEALTH_WAIT ]]; then
    err "Green container did not become healthy within ${MAX_HEALTH_WAIT}s."
    rollback
  fi
  sleep 2
  WAITED=$((WAITED + 2))
done
ok "Green is healthy after ${WAITED}s."

log "Updating nginx upstream to green (port ${GREEN_PORT})..."
write_upstream "$GREEN_PORT"
ok "Traffic now routed to green."

log "Settling for ${SETTLE_WAIT}s — watching green logs for errors..."
sleep "$SETTLE_WAIT"

ERROR_COUNT=$(docker logs --since "${SETTLE_WAIT}s" "$GREEN_CONTAINER" 2>&1 | grep -ciE '"level":50|"level":"error"|ERROR|FATAL' || true)
if [[ "$ERROR_COUNT" -gt 0 ]]; then
  warn "${ERROR_COUNT} error-level log entries detected in green during settle window."
  rollback
fi
log "Verification passed (0 errors in last ${SETTLE_WAIT}s)"

if [[ "$ACTIVE" == "blue" ]]; then
  log "Stopping blue container..."
  docker stop "$BLUE_CONTAINER" || true
  docker rm "$BLUE_CONTAINER" || true
  ok "Blue container stopped."
fi

log "Renaming green to blue for next cycle..."
docker rename "$GREEN_CONTAINER" "$BLUE_CONTAINER"

ok "Deployment complete. ${DEPLOY_IMAGE} is live on port ${BLUE_PORT}."