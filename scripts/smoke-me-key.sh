#!/usr/bin/env bash
# smoke-me-key.sh — verify the per-user Immich-key UX end-to-end via curl.
#
# This is the POSTSHIP-T11 (cycle 3) fallback when Chrome MCP is unavailable.
# It proves, against the deployed binary, the gap closed by POSTSHIP-T9+T10:
#
#   - An OIDC-authenticated user whose `immich_api_keys` row is missing gets
#     a deterministic `412 no_immich_key` from `/api/v1/me/people`, with a
#     `hint` body field that names `/me` (so the SPA's banner can link there).
#   - After that same user POSTs `/api/v1/me/immich-key` with a valid
#     base_url + key, `/api/v1/me/people` switches to `200` with a non-empty
#     array — i.e. the user's Immich people library is now reachable.
#
# Why a fallback exists: the local Chrome MCP harness isn't available on the
# burner, so a real browser session against the deployed immich-extended can't
# be driven by this iteration. A curl-and-cookiejar walk against the same
# deployed image catches the same class of bug (deployment drift, route
# regressions, missing scope mappings) at the API layer.
#
# Requirements (env vars, sourced from ~/code/immich-extended/.ralph/creds.env):
#   DOMAIN, AUTHENTIK_URL, SMOKE_USER, SMOKE_PASSWORD, IMMICH_BASE_URL,
#   IMMICH_ADMIN_KEY
#
# Exit codes:
#   0  both the 412-no-key path AND the populated-after-paste path passed
#   1  any stage failed (diagnostic context printed to stderr)
#
# This script never echoes credentials. Diagnostics show hashes/lengths only.

set -euo pipefail

: "${DOMAIN:?DOMAIN must be set (source creds.env)}"
: "${AUTHENTIK_URL:?AUTHENTIK_URL must be set}"
: "${SMOKE_USER:?SMOKE_USER must be set}"
: "${SMOKE_PASSWORD:?SMOKE_PASSWORD must be set}"
: "${IMMICH_BASE_URL:?IMMICH_BASE_URL must be set}"
: "${IMMICH_ADMIN_KEY:?IMMICH_ADMIN_KEY must be set}"

APP="https://immich-ext.${DOMAIN}"
UA='Mozilla/5.0 (smoke-me-key/1.0) curl'
JAR=$(mktemp /tmp/iext-me-jar.XXXXXX)
trap 'rm -f "$JAR" /tmp/iext-me-*.json /tmp/iext-me-*.html' EXIT

log() { printf '%s\n' "[smoke-me-key] $*" >&2; }
die() { printf '%s\n' "[smoke-me-key] FATAL: $*" >&2; exit 1; }

# ============================================================================
# stage A — OIDC login (same flow as smoke-oidc.sh; inlined to keep the
# script self-contained and to share one cookie jar across all subsequent
# /me/* calls).
# ============================================================================

log "stage A: OIDC login as SMOKE_USER (sha256_12=$(printf '%s' "$SMOKE_USER" | sha256sum | cut -c1-12))"

# A1. Kick off SSO. Server returns a 303 to Authentik.
auth_url=$(curl -sS -c "$JAR" -b "$JAR" -o /dev/null \
  -w '%{redirect_url}\n%{http_code}' \
  "$APP/api/v1/auth/oidc/login")
auth_redirect=$(printf '%s\n' "$auth_url" | head -1)
auth_code=$(printf '%s\n' "$auth_url" | tail -1)
[ "$auth_code" = "303" ] || die "A1: expected 303 from /oidc/login, got $auth_code"
case "$auth_redirect" in
  "${AUTHENTIK_URL}"/*) ;;
  *) die "A1: 303 redirect not on AUTHENTIK_URL: $auth_redirect" ;;
esac

# A2. Follow into the Authentik flow executor UI URL.
flow_final=$(curl -sS -c "$JAR" -b "$JAR" -L -A "$UA" -o /dev/null \
  -w '%{url_effective}' "$auth_redirect")
flow_slug=$(printf '%s' "$flow_final" | sed -nE 's|.*/if/flow/([^/]+)/.*|\1|p')
[ -n "$flow_slug" ] || die "A2: could not extract flow_slug from $flow_final"
flow_qs="${flow_final#*\?}"
enc_q=$(python3 -c 'import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))' "$flow_qs")
EXEC="${AUTHENTIK_URL}/api/v3/flows/executor/${flow_slug}/?query=$enc_q"

# A3. GET the first stage (expect ak-stage-identification).
stage=$(curl -sS -c "$JAR" -b "$JAR" -L -A "$UA" \
  -H 'Accept: application/json' -H "Referer: $flow_final" "$EXEC")
comp=$(printf '%s' "$stage" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("component",""))')
[ "$comp" = "ak-stage-identification" ] || die "A3: expected ak-stage-identification, got '$comp'"

# A4. Walk stages until xak-flow-redirect.
for i in 1 2 3 4 5 6 7 8; do
  case "$comp" in
    ak-stage-identification)
      body=$(python3 -c 'import json,os; print(json.dumps({"uid_field":os.environ["SMOKE_USER"],"component":"ak-stage-identification"}))') ;;
    ak-stage-password)
      body=$(python3 -c 'import json,os; print(json.dumps({"password":os.environ["SMOKE_PASSWORD"],"component":"ak-stage-password"}))') ;;
    ak-stage-user-login)
      body='{"remember_me":false,"component":"ak-stage-user-login"}' ;;
    xak-flow-redirect)
      to=$(printf '%s' "$stage" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("to",""))')
      [ -n "$to" ] || die "A4: redirect stage with empty 'to'"
      break ;;
    *)
      die "A4: unhandled component '$comp' at iter $i; body: $(printf '%s' "$stage" | head -c 600)" ;;
  esac
  stage=$(curl -sS -c "$JAR" -b "$JAR" -L -A "$UA" \
    -H 'Accept: application/json' -H 'Content-Type: application/json' \
    -H "Referer: $flow_final" \
    --data "$body" "$EXEC")
  comp=$(printf '%s' "$stage" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("component",""))')
  errs=$(printf '%s' "$stage" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("response_errors","") or "")')
  if [ -n "$errs" ] && [ "$errs" != "{}" ]; then
    die "A4: stage '$comp' returned errors: $errs"
  fi
done
[ "$comp" = "xak-flow-redirect" ] || die "A4: flow did not reach redirect (last comp: $comp)"

# A5. Follow redirect back to our /oidc/callback.
case "$to" in
  http://*|https://*) to_abs="$to" ;;
  /*) to_abs="${AUTHENTIK_URL}${to}" ;;
  *) die "A5: unexpected 'to' format: $to" ;;
esac
final=$(curl -sS -c "$JAR" -b "$JAR" -L -A "$UA" \
  -o /tmp/iext-me-callback.html \
  -w '%{url_effective}\n%{http_code}' "$to_abs")
final_url=$(printf '%s\n' "$final" | head -1)
final_code=$(printf '%s\n' "$final" | tail -1)
case "$final_url" in
  "${APP}/"|"${APP}"|"${APP}/?"*) ;;
  *) die "A5: expected to land at ${APP}/, got: $final_url" ;;
esac
[ "$final_code" = "200" ] || die "A5: final HTTP code $final_code, expected 200"

# A6. Confirm __Host-iext_session is in the jar.
grep -qE '#HttpOnly_immich-ext\..*__Host-iext_session' "$JAR" \
  || die "A6: __Host-iext_session cookie not set; jar: $(cat "$JAR")"
log "stage A PASS — session established"

# ============================================================================
# stage B — DELETE any pre-existing key so we exercise the 412 no_immich_key
# path deterministically. The endpoint is idempotent (204 either way).
# ============================================================================

log "stage B: DELETE /api/v1/me/immich-key (cleanup, idempotent)"
del_code=$(curl -sS -b "$JAR" -A "$UA" \
  -X DELETE -o /dev/null -w '%{http_code}' \
  "$APP/api/v1/me/immich-key")
[ "$del_code" = "204" ] || die "B: DELETE returned $del_code, expected 204"

# ============================================================================
# stage C — GET /me/people with no key, expect 412 + no_immich_key + hint.
# ============================================================================

log "stage C: GET /api/v1/me/people with no key (expect 412 no_immich_key)"
no_key_code=$(curl -sS -b "$JAR" -A "$UA" \
  -o /tmp/iext-me-people-nokey.json -w '%{http_code}' \
  "$APP/api/v1/me/people")
[ "$no_key_code" = "412" ] || die "C: expected 412, got $no_key_code (body: $(cat /tmp/iext-me-people-nokey.json | head -c 400))"

python3 - <<'PY' || die "C: response body did not match expected no_immich_key shape"
import json, sys
with open('/tmp/iext-me-people-nokey.json') as f:
    body = json.load(f)
err = body.get('error')
hint = body.get('hint', '')
if err != 'no_immich_key':
    print(f"expected error=no_immich_key, got {err!r}", file=sys.stderr); sys.exit(1)
if '/me' not in hint:
    print(f"expected hint to mention '/me', got {hint!r}", file=sys.stderr); sys.exit(1)
print(f"412 no_immich_key OK; hint mentions /me (len={len(hint)})")
PY

# ============================================================================
# stage D — POST /me/immich-key with IMMICH_BASE_URL + IMMICH_ADMIN_KEY.
# The server validates against /api/users/me on the upstream Immich, encrypts
# the plaintext with the master key, and UPSERTs.
# ============================================================================

log "stage D: POST /api/v1/me/immich-key (paste a real key, expect 200)"
paste_body=$(python3 -c 'import json,os; print(json.dumps({"base_url":os.environ["IMMICH_BASE_URL"],"api_key":os.environ["IMMICH_ADMIN_KEY"]}))')
paste_code=$(curl -sS -b "$JAR" -A "$UA" \
  -H 'Content-Type: application/json' \
  -X POST --data "$paste_body" \
  -o /tmp/iext-me-paste.json -w '%{http_code}' \
  "$APP/api/v1/me/immich-key")
[ "$paste_code" = "200" ] || die "D: POST returned $paste_code (body: $(cat /tmp/iext-me-paste.json | head -c 400))"

python3 - <<'PY' || die "D: paste response missing required fields"
import json, sys
with open('/tmp/iext-me-paste.json') as f:
    body = json.load(f)
for k in ('base_url', 'immich_user_id', 'last_validated_at'):
    if k not in body or body[k] in (None, '', 0):
        print(f"missing/empty field {k!r}: {body!r}", file=sys.stderr); sys.exit(1)
print(f"paste OK; immich_user_id={body['immich_user_id']} last_validated_at={body['last_validated_at']}")
PY

# ============================================================================
# stage E — GET /me/people again, expect 200 + non-empty array.
# ============================================================================

log "stage E: GET /api/v1/me/people with paste'd key (expect 200 + non-empty)"
ok_code=$(curl -sS -b "$JAR" -A "$UA" \
  -o /tmp/iext-me-people-ok.json -w '%{http_code}' \
  "$APP/api/v1/me/people")
[ "$ok_code" = "200" ] || die "E: expected 200, got $ok_code (body: $(cat /tmp/iext-me-people-ok.json | head -c 400))"

python3 - <<'PY' || die "E: populated /me/people response failed validation"
import json, sys
with open('/tmp/iext-me-people-ok.json') as f:
    people = json.load(f)
if not isinstance(people, list):
    print(f"expected JSON array, got {type(people).__name__}", file=sys.stderr); sys.exit(1)
if len(people) == 0:
    print("expected non-empty array (Immich library has no identifiable people?)", file=sys.stderr); sys.exit(1)
# Sample first item for shape correctness without echoing names.
p = people[0]
for k in ('id', 'name', 'thumbnail_url'):
    if k not in p:
        print(f"missing field {k!r} in first person: {p!r}", file=sys.stderr); sys.exit(1)
if not p['thumbnail_url'].startswith('/api/v1/me/people/'):
    print(f"thumbnail_url does not route through our proxy: {p['thumbnail_url']!r}", file=sys.stderr); sys.exit(1)
print(f"populated OK; len={len(people)} first.id={p['id'][:8]}...")
PY

log "smoke-me-key PASS — both 412 (no key) and 200 (after paste) paths verified"
