#!/bin/bash
# One-shot bootstrap: waits for both servers, seeds the account the Workstation
# image signs requests with, and publishes the matching key in the shared
# volume. Re-running it with the same volumes is a no-op.
set -euo pipefail

NODE="${NODE_NAME:-workstation}"
ORG="${MICROWAVE_ORG:-demo}"
KEYS=/keys
DATABASE_URL="postgres://supermarket:supermarket@db:5432/supermarket"

log() { printf 'bootstrap: %s\n' "$*"; }

wait_for() {
  local url=$1 name=$2 tries=${3:-60}
  for _ in $(seq "$tries"); do
    if curl -fsS --max-time 2 "$url" >/dev/null 2>&1; then
      log "$name is up"
      return 0
    fi
    sleep 1
  done
  log "$name did not answer $url"
  return 1
}

wait_for "http://microwave:8000/_status" microwave
wait_for "http://supermarket:3000/api/v1/health" supermarket

mkdir -p "$KEYS"

if [ ! -s "$KEYS/$NODE.pem" ]; then
  log "generating $NODE keypair"
  openssl genrsa -out "$KEYS/$NODE.pem" 2048 2>/dev/null
  chmod 600 "$KEYS/$NODE.pem"
fi
openssl rsa -in "$KEYS/$NODE.pem" -pubout -out "$KEYS/$NODE.pub" 2>/dev/null
chmod 644 "$KEYS/$NODE.pub"

# The Microwave CLI is not idempotent itself; the marker makes a re-run cheap.
if [ ! -e "$KEYS/.microwave-ready" ]; then
  if ! microwave-ctl user-show "$NODE" >/dev/null 2>&1; then
    log "creating server user $NODE"
    microwave-ctl user-create "$NODE" --email "$NODE@example.test" --admin \
      --public-key-path "$KEYS/$NODE.pub" >/dev/null
  fi
  if ! microwave-ctl org-show "$ORG" >/dev/null 2>&1; then
    log "creating organization $ORG"
    microwave-ctl org-create "$ORG" --full-name "Demo" --association-user "$NODE" >/dev/null
  fi
  microwave-ctl org-user-add "$ORG" "$NODE" >/dev/null 2>&1 || true
  microwave-ctl add-client-key "$ORG" "$ORG-validator" --public-key-path "$KEYS/$NODE.pub" \
    >/dev/null 2>&1 \
    || { keys=$(microwave-ctl list-client-keys "$ORG" "$ORG-validator"); grep -q '^name:' <<<"$keys"; }
  microwave-ctl add-user-key "$NODE" --public-key-path "$KEYS/$NODE.pub" \
    >/dev/null 2>&1 \
    || { keys=$(microwave-ctl list-user-keys "$NODE"); grep -q '^name:' <<<"$keys"; }
  touch "$KEYS/.microwave-ready"
  log "server user and organization ready"
fi

# Supermarket account whose login is the Workstation's node name and whose
# public key verifies the signed upload. `accounts.username` is the mapping.
user_id=$(psql "$DATABASE_URL" -qtAc \
  "SELECT user_id FROM accounts WHERE provider = 'chef_oauth2' AND username = '$NODE'")
if [ -z "$user_id" ]; then
  log "creating Supermarket user $NODE"
  user_id=$(psql "$DATABASE_URL" -qtAc \
    "WITH ins AS (
       INSERT INTO users (first_name, last_name, created_at, updated_at, email, roles_mask)
       VALUES ('Demo', 'Workstation', now(), now(), '$NODE@example.test', 1) RETURNING id)
     SELECT id FROM ins")
fi

psql "$DATABASE_URL" -q -v ON_ERROR_STOP=1 -v user_id="$user_id" \
  -v node="$NODE" -v pub="$(cat "$KEYS/$NODE.pub")" <<'SQL'
UPDATE users SET public_key = :'pub', roles_mask = 1, updated_at = now() WHERE id = :user_id;
INSERT INTO accounts (user_id, uid, username, oauth_token, oauth_expires, provider)
VALUES (:user_id, :'node', :'node', 'imported', now() + interval '10 years', 'chef_oauth2')
ON CONFLICT (uid, provider) DO UPDATE
  SET user_id = EXCLUDED.user_id, username = EXCLUDED.username;
INSERT INTO categories (name, created_at, updated_at, slug)
SELECT 'Other', now(), now(), 'other'
WHERE NOT EXISTS (SELECT 1 FROM categories WHERE name = 'Other');
SQL

log "ready: node=$NODE org=$ORG"
