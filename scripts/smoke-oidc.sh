#!/usr/bin/env bash
# smoke-oidc.sh — end-to-end OIDC login smoke against the deployed immich-extended.
#
# Drives a full SSO login from curl alone, walking Authentik's flow executor
# JSON API. Used by POSTSHIP-T7 to live-verify that the deployed binary
# completes the OIDC callback (issues a __Host-iext_session cookie + populates
# the user row + returns the user on GET /api/v1/auth/me).
#
# Why this script exists: the in-tree integration tests
# (crates/server/tests/oidc.rs) cover the server logic against a wiremock IdP.
# They cannot catch deployment drift — image not rebuilt, env var mistypes,
# Authentik provider misconfig. A real curl-driven login against the real
# Authentik catches that class of bug.
#
# Requirements (env vars, sourced from ~/code/immich-extended/.ralph/creds.env):
#   DOMAIN, AUTHENTIK_URL, SMOKE_USER, SMOKE_PASSWORD
#
# Exit codes:
#   0  full login succeeded; __Host-iext_session set; /me returned populated user
#   1  any stage failed (each failure prints diagnostic context to stderr)
#
# This script intentionally does NOT echo SMOKE_PASSWORD or session cookie
# values to stdout. Diagnostics print only structural attrs / lengths / hashes.
set -euo pipefail

: "${DOMAIN:?DOMAIN must be set (source creds.env)}"
: "${AUTHENTIK_URL:?AUTHENTIK_URL must be set}"
: "${SMOKE_USER:?SMOKE_USER must be set}"
: "${SMOKE_PASSWORD:?SMOKE_PASSWORD must be set}"

APP="https://immich-ext.${DOMAIN}"
UA='Mozilla/5.0 (smoke-oidc/1.0) curl'
JAR=$(mktemp /tmp/iext-smoke-jar.XXXXXX)
trap 'rm -f "$JAR"' EXIT

log() { printf '%s\n' "[smoke-oidc] $*" >&2; }
die() { printf '%s\n' "[smoke-oidc] FATAL: $*" >&2; exit 1; }

# --- step 1: kick off SSO from immich-extended ------------------------------
log "step 1: GET ${APP}/api/v1/auth/oidc/login"
auth_url=$(curl -sS -c "$JAR" -b "$JAR" -o /dev/null \
  -w '%{redirect_url}\n%{http_code}' \
  "$APP/api/v1/auth/oidc/login")
auth_redirect=$(printf '%s\n' "$auth_url" | head -1)
auth_code=$(printf '%s\n' "$auth_url" | tail -1)
[ "$auth_code" = "303" ] || die "expected 303, got $auth_code"
case "$auth_redirect" in
  "${AUTHENTIK_URL}"/*) ;;
  *) die "303 location not on AUTHENTIK_URL: $auth_redirect" ;;
esac

# --- step 2: follow into Authentik flow executor UI -------------------------
log "step 2: follow → /if/flow/<slug>/"
flow_final=$(curl -sS -c "$JAR" -b "$JAR" -L -A "$UA" -o /dev/null \
  -w '%{url_effective}' "$auth_redirect")
flow_slug=$(printf '%s' "$flow_final" | sed -nE 's|.*/if/flow/([^/]+)/.*|\1|p')
[ -n "$flow_slug" ] || die "could not extract flow_slug from $flow_final"

flow_qs="${flow_final#*\?}"
enc_q=$(python3 -c 'import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))' "$flow_qs")
EXEC="${AUTHENTIK_URL}/api/v3/flows/executor/${flow_slug}/?query=$enc_q"

# --- step 3: GET first stage (identification) -------------------------------
log "step 3: GET executor → first stage"
stage=$(curl -sS -c "$JAR" -b "$JAR" -L -A "$UA" \
  -H 'Accept: application/json' -H "Referer: $flow_final" "$EXEC")
comp=$(printf '%s' "$stage" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("component",""))')
[ "$comp" = "ak-stage-identification" ] || die "expected ak-stage-identification, got '$comp'"

# --- step 4: walk the flow stages until we get a redirect off Authentik ----
for i in 1 2 3 4 5 6 7 8; do
  case "$comp" in
    ak-stage-identification)
      log "step 4.$i: POST identification"
      body=$(python3 -c 'import json,os; print(json.dumps({"uid_field":os.environ["SMOKE_USER"],"component":"ak-stage-identification"}))')
      ;;
    ak-stage-password)
      log "step 4.$i: POST password"
      body=$(python3 -c 'import json,os; print(json.dumps({"password":os.environ["SMOKE_PASSWORD"],"component":"ak-stage-password"}))')
      ;;
    ak-stage-user-login)
      log "step 4.$i: POST user-login (no-op consent)"
      body='{"remember_me":false,"component":"ak-stage-user-login"}'
      ;;
    xak-flow-redirect)
      to=$(printf '%s' "$stage" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("to",""))')
      [ -n "$to" ] || die "redirect stage with empty 'to'"
      log "step 4.$i: redirect off Authentik → $(printf '%s' "$to" | sed -E 's|\?.*|?…|')"
      break
      ;;
    *)
      die "unhandled component '$comp' at iter $i; body: $(printf '%s' "$stage" | head -c 600)"
      ;;
  esac
  # POST and follow the 302 chain that lands on the next stage's GET.
  stage=$(curl -sS -c "$JAR" -b "$JAR" -L -A "$UA" \
    -H 'Accept: application/json' -H 'Content-Type: application/json' \
    -H "Referer: $flow_final" \
    --data "$body" "$EXEC")
  comp=$(printf '%s' "$stage" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("component",""))')
  errs=$(printf '%s' "$stage" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("response_errors","") or "")')
  if [ -n "$errs" ] && [ "$errs" != "{}" ]; then
    die "stage '$comp' returned errors: $errs"
  fi
done
[ "$comp" = "xak-flow-redirect" ] || die "flow did not reach redirect (last comp: $comp)"

# --- step 5: follow the redirect off Authentik back to our callback --------
# The 'to' field is usually a relative path (e.g. /application/o/authorize/?...);
# resolve it against AUTHENTIK_URL. That URL itself 302s to our callback at
# https://immich-ext.${DOMAIN}/api/v1/auth/oidc/callback?code=...&state=...
case "$to" in
  http://*|https://*) to_abs="$to" ;;
  /*) to_abs="${AUTHENTIK_URL}${to}" ;;
  *) die "unexpected 'to' format: $to" ;;
esac
log "step 5: follow redirect chain back to immich-ext callback"
final=$(curl -sS -c "$JAR" -b "$JAR" -L -A "$UA" \
  -o /tmp/iext-smoke-callback-body.html \
  -w '%{url_effective}\n%{http_code}' "$to_abs")
final_url=$(printf '%s\n' "$final" | head -1)
final_code=$(printf '%s\n' "$final" | tail -1)
log "step 5 final: $final_url HTTP=$final_code"

case "$final_url" in
  "${APP}/"|"${APP}"|"${APP}/?"*) log "landed at app root, as expected" ;;
  *) die "expected to land at ${APP}/, got: $final_url" ;;
esac
[ "$final_code" = "200" ] || die "final HTTP code was $final_code, expected 200"

# --- step 6: verify __Host-iext_session cookie was set with correct attrs --
log "step 6: inspect cookie jar"
# Curl cookie jar format: domain tab tailmatch tab path tab secure tab expiry tab name tab value
# httpOnly flag is encoded by prepending '#HttpOnly_' to the domain field.
cookie_line=$(grep -E '#HttpOnly_immich-ext\.|^immich-ext\.' "$JAR" | grep '__Host-iext_session' || true)
[ -n "$cookie_line" ] || die "no __Host-iext_session in jar; full jar: $(cat "$JAR")"

# Extract fields tab-separated.
IFS=$'\t' read -r c_dom c_tail c_path c_secure c_exp c_name c_val <<<"$cookie_line"
case "$c_dom" in
  '#HttpOnly_'*) httponly=1; bare_dom="${c_dom#'#HttpOnly_'}" ;;
  *) httponly=0; bare_dom="$c_dom" ;;
esac

log "cookie attrs:"
log "  name      = $c_name"
log "  domain    = $bare_dom"
log "  path      = $c_path"
log "  secure    = $c_secure"
log "  httponly  = $httponly"
log "  value_len = ${#c_val} bytes"

[ "$c_path" = '/' ] || die "__Host- cookie must have Path=/, got $c_path"
[ "$c_secure" = 'TRUE' ] || die "__Host- cookie must be Secure, got $c_secure"
[ "$httponly" = '1' ] || die "__Host- cookie must be HttpOnly"
# Curl writes the actual host (not the .domain) for __Host- prefixed cookies
# because the Set-Cookie header cannot carry a Domain= attribute.
[ "$bare_dom" = "immich-ext.${DOMAIN}" ] || die "domain unexpected: $bare_dom"

# --- step 7: drive /api/v1/auth/me with the session ------------------------
log "step 7: GET /api/v1/auth/me with session"
me=$(curl -sS -b "$JAR" -A "$UA" -o /tmp/iext-smoke-me.json -w '%{http_code}' "$APP/api/v1/auth/me")
[ "$me" = "200" ] || die "/api/v1/auth/me returned $me (body: $(cat /tmp/iext-smoke-me.json | head -c 400))"

# Validate response shape WITHOUT echoing PII (email + display_name).
python3 - <<'PY'
import json, sys, hashlib
with open('/tmp/iext-smoke-me.json') as f:
    d = json.load(f)
required = ['user_id', 'email', 'display_name']
missing = [k for k in required if not d.get(k)]
if missing:
    print(f"missing required fields in /me: {missing}", file=sys.stderr)
    sys.exit(1)
# Print SHA-256 prefixes of PII fields as evidence without leaking values.
print(f"user_id={d['user_id']}")  # not PII — internal UUID
for f in ('email', 'display_name'):
    h = hashlib.sha256(d[f].encode()).hexdigest()[:12]
    print(f"{f}_sha256_12={h} len={len(d[f])}")
PY

log "smoke-oidc PASS"
