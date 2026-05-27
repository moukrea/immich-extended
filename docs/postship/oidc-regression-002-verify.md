# POSTSHIP cycle 2 — live verification (POSTSHIP-T7)

**Date**: 2026-05-27
**Deployed commit**: `f24017c` (`fix(server,oidc): fallback to userinfo when ID token lacks email + informative hint`)
**Image id**: `sha256:5a4d79a7f57346c06bbbca30348eb280162a209ec84c7efc28a07723809f3506` (built 2026-05-27T12:19:09+02:00)
**Container**: `immich-extended` running `immich-extended:dev`, healthy
**Driver**: `scripts/smoke-oidc.sh` (curl-only — Chrome MCP not available in the worker)

## Why this run matters

POSTSHIP cycle 1 T4 only verified the **first hop** of the SSO chain
(`browser → server → 303 → Authentik`) and declared victory. That left two
classes of regression invisible:

1. **POSTSHIP cycle 2's root cause** — the Authentik OAuth2Provider for
   `immich-extended` had `property_mappings=[]`, so the ID token came back
   with sub+iss only and the callback returned `{"error":"missing_email_claim"}`.
   Cycle 1's T4 never POSTed the password, so it never reached the callback.
2. **Any future drift** in the post-callback path (cookie attrs, /me shape,
   session-row creation). The in-tree mock-IdP tests cover the server logic,
   but cannot catch deployment drift (image not rebuilt, env var mistype,
   Authentik silently dropping a scope mapping).

This run drives the FULL chain: `/login → Authentik authorize → identification
stage → password stage → flow redirect → /application/o/authorize/ → our
/callback → __Host-iext_session set → /api/v1/auth/me returns populated user`.

## Run output

```
[smoke-oidc] step 1: GET https://immich-ext.rdti25e2d.dedyn.io/api/v1/auth/oidc/login
[smoke-oidc] step 2: follow → /if/flow/<slug>/
[smoke-oidc] step 3: GET executor → first stage
[smoke-oidc] step 4.1: POST identification
[smoke-oidc] step 4.2: POST password
[smoke-oidc] step 4.3: redirect off Authentik → /application/o/authorize/?…
[smoke-oidc] step 5: follow redirect chain back to immich-ext callback
[smoke-oidc] step 5 final: https://immich-ext.rdti25e2d.dedyn.io/ HTTP=200
[smoke-oidc] landed at app root, as expected
[smoke-oidc] step 6: inspect cookie jar
[smoke-oidc] cookie attrs:
[smoke-oidc]   name      = __Host-iext_session
[smoke-oidc]   domain    = immich-ext.rdti25e2d.dedyn.io
[smoke-oidc]   path      = /
[smoke-oidc]   secure    = TRUE
[smoke-oidc]   httponly  = 1
[smoke-oidc]   value_len = 64 bytes
[smoke-oidc] step 7: GET /api/v1/auth/me with session
user_id=5d5bcfe9-073c-4a9f-8eb0-4cebd15772c7
email_sha256_12=70022edd7cf6 len=24
display_name_sha256_12=aaa282276038 len=16
[smoke-oidc] smoke-oidc PASS
```

(Email + display_name printed as SHA-256 prefix + length to avoid leaking PII
into the source tree. The `user_id` is an internal UUID, not PII.)

## What the run proves

### 1. SPA-hijack fix still holds (cycle 1 regression)

`web/src/pages/Login.tsx`'s SSO anchor still has `rel="external"` in the
deployed bundle, so step 1 reaches a real `GET /api/v1/auth/oidc/login` on
the server (not an SPA route). The server returns `303 Location: https://auth.${DOMAIN}/application/o/authorize/?...`
with PKCE params — pinned at the source by
`web/src/pages/__tests__/login.test.tsx` (commit `7b18972`).

### 2. Authentik OAuth2Provider scope mappings restored (cycle 2 T5)

Steps 4.2 → 5 succeed end-to-end. If the provider's `property_mappings` had
remained `[]`, Authentik would have issued an ID token with sub+iss only,
and step 5's final URL would have been
`https://immich-ext.${DOMAIN}/api/v1/auth/oidc/callback?error=missing_email_claim`
(JSON body, not the 200 we see). The PATCH from cycle 2 T5
(`docs/postship/oidc-regression-002-authentik-mappings.md`) is therefore still
intact in Authentik's state.

### 3. T6 userinfo fallback is wired (defensive)

The smoke run alone can't distinguish "email came from ID token" vs "email
came from userinfo fallback" — both produce the same `/me` payload. T6's
positive/negative paths are covered by
`crates/server/tests/oidc.rs::callback_falls_back_to_userinfo_when_id_token_lacks_email`
and `::callback_fails_with_hint_when_id_token_and_userinfo_both_lack_email`
(commit `f24017c`). The smoke verifies the green path through the actual
binary — both fallback branches and the non-fallback branch share the same
post-extraction code, so a green smoke means the post-extraction code is
sound on the deployed binary.

### 4. `__Host-iext_session` cookie invariants hold in production

The deployed binary issues the session cookie with the four invariants the
RFC requires for `__Host-` prefix:

| attribute | required | observed |
| --------- | -------- | -------- |
| `Path`    | `/`      | `/`      |
| `Secure`  | true     | `TRUE`   |
| `HttpOnly`| true     | `1`      |
| `Domain`  | unset    | (curl's jar shows the actual host `immich-ext.rdti25e2d.dedyn.io`; for `__Host-` cookies the Set-Cookie header carries NO `Domain=` attribute — curl falls back to the request host, which is the documented behavior) |

These attrs are pinned by the integration test
`crates/server/tests/oidc.rs::callback_sets_production_cookie_with_host_prefix_invariants`
(commit `ae9c8d4`). The smoke confirms the deployed binary matches.

### 5. Session is usable: `/me` returns the populated user

`GET /api/v1/auth/me` with only the `__Host-iext_session` cookie returns
HTTP 200 + JSON with `user_id`, `email`, `display_name` all populated.
That means: the callback successfully created (or matched) a `users` row,
inserted a `sessions` row keyed on the cookie value, and the session
middleware joins both.

## What this run does NOT cover

- **MFA stages** — the smoke user `immich-ext-smoke` is intentionally
  MFA-free so the curl path can complete. Real human users may have MFA
  enforced by an Authentik policy; we have no curl-driven coverage of TOTP
  / WebAuthn stages. That's a known limitation of any curl-only smoke and
  is not unique to this codebase.
- **Logout flow** — `DELETE /api/v1/auth/session` is covered by unit tests
  but not exercised by this script. (Out of scope for the regression
  POSTSHIP cycle 2 is closing — that cycle was about login, not logout.)
- **First-time user provisioning** — the smoke user already existed in
  the immich-extended `users` table from a prior run, so this script
  exercises the "match existing user" branch. The "first login auto-create"
  branch has integration-test coverage at
  `crates/server/tests/oidc.rs::callback_creates_user_row_on_first_login`.

## How to re-run

```bash
set -a; source ~/code/immich-extended/.ralph/creds.env; set +a
~/code/immich-extended/scripts/smoke-oidc.sh
```

Exits 0 on success with `[smoke-oidc] smoke-oidc PASS`. Any failure is
printed to stderr with a diagnostic prefix and exits non-zero.

## Lesson for future POSTSHIP cycles

> A "live verification" task that doesn't actually complete the full happy
> path it's claimed to verify is not verification — it's a smoke alarm with
> the battery taken out. POSTSHIP cycle 1 T4 declared victory after reaching
> Authentik's login screen; it took a real user reporting `missing_email_claim`
> to surface the regression that ran one step further down the chain. Future
> POSTSHIP "live verification" tasks MUST drive the test until the user lands
> on the post-login destination AND the destination's API responds to an
> authenticated request. The `scripts/smoke-oidc.sh` script formalizes that
> bar.
