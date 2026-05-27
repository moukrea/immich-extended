# OIDC login regression #002 — Authentik OAuth2Provider scope mappings restored

- **Date**: 2026-05-27
- **Closes**: POSTSHIP-T5 (Authentik config drift fix; T6 adds defense-in-depth, T7 adds live login proof)
- **Symptom**: after the cycle-1 SPA-hijack fix landed, clicking SSO successfully reached Authentik, the user logged in, Authentik redirected back to `/api/v1/auth/oidc/callback?code=...&state=...`, and the immich-extended server responded with JSON `{"error":"missing_email_claim"}` from `crates/server/src/auth/oidc.rs:325-331`. The user noted that the same Authentik instance signs them into Immich without issue, so the bug was per-provider config drift, not a user/account problem.
- **Reference commits**: `e222f94` (close of cycle 1), `3618fe4` / `7b18972` / `ae9c8d4` (the cycle-1 fix train).

## Root cause

Authentik's `OAuth2Provider.property_mappings` is an **allow-list** for which scope→claim mappings the provider will honor at token issuance. The client requesting `scope=openid email profile` in the authorize URL is necessary but **not sufficient**; if those scope mappings are not attached to the provider via `property_mappings`, Authentik silently issues an ID token with only `sub` and `iss` populated.

For the immich-extended provider (`pk=2`, `client_id=qCgT…`), `property_mappings` was `[]`. For comparison, the Immich provider (which the user confirmed works) had the three default scope mappings shipped by Authentik. The asymmetry is invisible from the client side because the authorize redirect, login flow, and callback exchange all complete normally — the missing claims only surface when the server decodes the ID token and finds `email` absent.

The server-side claim extraction itself is correct (it has been since M1):

```rust
// crates/server/src/auth/oidc.rs:325-331
let email = claims
    .email()
    .ok_or_else(|| oidc_bad_request("missing_email_claim"))?
    .as_str()
    .to_string();
```

POSTSHIP-T6 adds a userinfo fallback so a misconfigured IdP yields a more actionable error code + hint rather than failing closed; this T5 fix removes the immediate misconfiguration so logins work in the meantime.

## Discovery — which mappings to attach

Authentik ships three default scope mappings for OIDC providers under managed paths `goauthentik.io/providers/oauth2/scope-{openid,email,profile}`. Discover their UUIDs at any time with:

```bash
set -a; source ~/code/immich-extended/.ralph/creds.env; set +a
curl -fsS "${AUTHENTIK_URL}/api/v3/propertymappings/provider/scope/?managed__startswith=goauthentik.io/providers/oauth2/scope-" \
  -H "Authorization: Bearer ${AUTHENTIK_BOOTSTRAP_TOKEN}" \
  | python3 -c 'import sys,json;[print(r["pk"],r["managed"]) for r in json.load(sys.stdin)["results"]]'
```

Result on this deployment (2026-05-27, recorded so future audits don't need to re-derive):

| Scope     | UUID                                   | Managed identifier                                  |
| --------- | -------------------------------------- | --------------------------------------------------- |
| `openid`  | `909f01a6-8149-4de6-87f4-8495e12729ff` | `goauthentik.io/providers/oauth2/scope-openid`      |
| `email`   | `0ecde820-03d8-49cd-823f-c0c0f79f0ad2` | `goauthentik.io/providers/oauth2/scope-email`       |
| `profile` | `ea77e6b6-9642-43e2-838b-af30385cba01` | `goauthentik.io/providers/oauth2/scope-profile`     |

The managed identifiers are stable across Authentik versions; the UUIDs are stable per Authentik installation (regenerated only if mappings are deleted and recreated).

## Fix — PATCH the immich-extended provider

```bash
set -a; source ~/code/immich-extended/.ralph/creds.env; set +a
curl -fsSX PATCH "${AUTHENTIK_URL}/api/v3/providers/oauth2/2/" \
  -H "Authorization: Bearer ${AUTHENTIK_BOOTSTRAP_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"property_mappings":["909f01a6-8149-4de6-87f4-8495e12729ff","0ecde820-03d8-49cd-823f-c0c0f79f0ad2","ea77e6b6-9642-43e2-838b-af30385cba01"]}'
```

The PATCH is idempotent — re-running with the same body returns 200 with no diff. **No Authentik restart is needed**; the next authorize request immediately uses the new mapping list.

### PATCH response (HTTP 200) — sensitive fields redacted

```json
{
  "pk": 2,
  "name": "immich-extended",
  "authorization_flow": "ce932684-43f3-4be2-bfb5-02171279939b",
  "invalidation_flow": "714ad51e-bc3c-47c6-a79e-78c4e5bbf66c",
  "property_mappings": [
    "909f01a6-8149-4de6-87f4-8495e12729ff",
    "0ecde820-03d8-49cd-823f-c0c0f79f0ad2",
    "ea77e6b6-9642-43e2-838b-af30385cba01"
  ],
  "component": "ak-provider-oauth2-form",
  "assigned_application_slug": "immich-extended",
  "verbose_name": "OAuth2/OpenID Provider",
  "client_type": "confidential",
  "client_id": "qCgTGZWH5Zy1pGxXibCum3tvbsAkOJ0CcX4ZIiv8",
  "client_secret": "<REDACTED — see creds.env IMMICH_EXT_OIDC_CLIENT_SECRET>",
  "include_claims_in_id_token": true,
  "signing_key": "cd485ffc-0cc8-4132-aea3-cd4a0cbec79c",
  "redirect_uris": [
    {
      "matching_mode": "strict",
      "url": "https://immich-ext.rdti25e2d.dedyn.io/api/v1/auth/oidc/callback"
    }
  ],
  "sub_mode": "hashed_user_id",
  "issuer_mode": "per_provider"
}
```

The `property_mappings` array is the load-bearing change: it went from `[]` → the three default scope-mapping UUIDs. Every other field is unchanged.

## Verification — curl smoke (partial, by design)

T5 owns the config fix; T7 owns the browser-driven end-to-end login proof. The smoke below is what T5 contributes:

1. **`GET /api/v1/auth/oidc/login` still 303s to Authentik authorize with the expected scope set**, post-PATCH:

   ```
   HTTP=303
   Location: https://auth.${DOMAIN}/application/o/authorize/?response_type=code
              &client_id=qCgTGZWH5Zy1pGxXibCum3tvbsAkOJ0CcX4ZIiv8
              &state=<REDACTED>
              &code_challenge=<REDACTED>
              &code_challenge_method=S256
              &redirect_uri=https%3A%2F%2Fimmich-ext.rdti25e2d.dedyn.io%2Fapi%2Fv1%2Fauth%2Foidc%2Fcallback
              &scope=openid+openid+email+profile
              &nonce=<REDACTED>
   ```

   (The duplicated `openid` is harmless — OIDC spec treats `scope` as a set. It comes from `openidconnect` always inserting `openid` plus the per-call list we already include `openid` in; cosmetic, not blocking. Tracked for cleanup independently of this regression.)

2. **Authentik's flow executor advances past the identification stage** when we POST the bootstrap email (driving `/api/v3/flows/executor/default-authentication-flow/`). Response transitions `ak-stage-identification` → `ak-stage-password` with `pending_user: akadmin`, which proves the OAuth2 flow + provider association are still wired correctly through Authentik.

3. **Full login was not driven from curl** because `AUTHENTIK_BOOTSTRAP_PASSWORD` in `creds.env` has been rotated since first boot (the password stage returns `{"code":"invalid","string":"Invalid password"}` for that value). Rotating it back would have been disruptive (it is the operator-owned admin password, not a worker-owned credential). T7 drives a real login via Chrome MCP / headless Chromium against the operator's actual session — that is the right place to prove the callback now lands at `/` with a populated `__Host-iext_session`.

## Re-entry — what T7 must assert

After T6 lands (defensive userinfo fallback + tests), T7 must drive:

- `https://immich-ext.${DOMAIN}/login` → SSO click → Authentik login (real password, via operator-driven browser) → `https://immich-ext.${DOMAIN}/` (the SPA Dashboard route, NOT the JSON callback URL).
- `__Host-iext_session` cookie present with `Secure`, `HttpOnly`, `SameSite=Lax`, `Path=/`, no `Domain`.
- `GET /api/v1/auth/me` from that cookie jar returns 200 with `{user_id, email, display_name}` populated — `email` populated specifically demonstrates that the mapping fix took effect end-to-end.

## Watch-out for M7-T3 (Authentik provisioning recipe)

The `M7-T3` task block in `TASKS.md` covers creating the Authentik OAuth2 provider + application from scratch via API. POSTSHIP-T8 will add an explicit "scope mappings PATCH" step to that recipe so any future bootstrap (e.g. on a clean Authentik) attaches `property_mappings` immediately rather than relying on an editor to know about this gotcha. The discovery query and the three UUIDs above are the canonical references.

Summary of the trap, for future readers: **after `POST /api/v3/providers/oauth2/`, follow up with a `PATCH /api/v3/providers/oauth2/<pk>/` to populate `property_mappings`. Cross-check against any working app in the same Authentik (e.g. Immich) — empty `property_mappings` means the provider issues stripped ID tokens regardless of the client's `scope` parameter.**
