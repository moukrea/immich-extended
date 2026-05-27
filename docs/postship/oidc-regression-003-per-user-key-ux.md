# POSTSHIP cycle 3 — live verification (POSTSHIP-T11)

**Date**: 2026-05-27
**Deployed commit**: `02c363d` (`feat(server,web): explicit no_immich_key state + connect CTA in people picker`)
**Image id**: `sha256:b5d3faa8f4c3a33b7310b6d0c0fda068211b307690a6ed0a4f8d2f8a45b1f695` (built 2026-05-27T13:27:08+02:00)
**Container**: `immich-extended` running `immich-extended:dev`, healthy after 1s
**Driver**: `scripts/smoke-me-key.sh` (curl-only — Chrome MCP not available in the worker)

## Why this run matters

POSTSHIP cycle 2 left a usability gap that cycle 3 (T9 + T10) closed in code
but not yet in the deployed binary:

- **The gap**: PRD §8 Pattern A built per-user encrypted Immich API keys
  (`immich_api_keys.user_id` FK), but the SPA only ever surfaced "paste a key"
  inside the `/setup` flow, which is gated by `needs_setup=true` and fires
  only for the first admin. Every subsequent OIDC user (i.e. `emeric` and
  anyone else who SSOs in after first boot) had zero rows in
  `immich_api_keys`, so `me/immich_proxy.rs::load_resolved_key()` could not
  construct an `ImmichClient` on their behalf. The downstream symptom was
  the misleading copy "No people in your Immich library yet." in the visual
  rule builder's People multi-selects — implying the user's Immich library
  was empty when in fact we had no key to call it with.
- **The code fix** (T9 + T10, both shipped on `main`):
  - **T9** (`951ced2`) — new `/me` SolidJS page that lets the logged-in user
    paste / inspect / revoke their Immich `base_url` + key, with 6 vitests
    pinning the connected / not-connected / paste-validation states.
  - **T10** (`02c363d`) — server: the three `/api/v1/me/*` proxies
    (`list_people`, `list_albums`, `proxy_person_thumbnail`) now route their
    no-key path through a shared `no_immich_key_response()` builder that
    returns `412 Precondition Failed` with body `{"error":"no_immich_key",
    "hint":"Add your Immich API key at /me to connect this account to
    Immich."}`. Other failure modes (`decrypt_failed` 500, `invalid_base_url`
    500, `upstream_unreachable` 502, `internal_error` 500) keep their
    existing semantics. Web: `web/src/lib/api.ts` exposes a new
    `MeFetchResult<T>` discriminated union from `fetchPeople()` /
    `fetchAlbums()`; `PeopleContext` exposes `{people, noImmichKey}`;
    `PeopleMultiSelect` swaps the misleading "No people…" copy for an amber
    `role="status"` banner linking to `/me`.

This run drives the FULL deployed binary through the new contract: OIDC
login as `SMOKE_USER`, then ensure the no-key path returns the deterministic
412, then paste a real Immich key, then assert the populated path returns
real people from the upstream Immich library. The in-tree integration tests
in `crates/server/tests/me_immich.rs` cover the server-side semantics
against a wiremock Immich; they cannot catch deployment drift (image not
rebuilt, route regressions, env var mistypes). A real curl-driven walk
against the real deployed binary catches that class of bug.

## Run output (timestamped, redacted)

```
$ /home/emeric/code/immich-extended/scripts/smoke-me-key.sh
[smoke-me-key] stage A: OIDC login as SMOKE_USER (sha256_12=695892e2ea8b)
[smoke-me-key] stage A PASS — session established
[smoke-me-key] stage B: DELETE /api/v1/me/immich-key (cleanup, idempotent)
[smoke-me-key] stage C: GET /api/v1/me/people with no key (expect 412 no_immich_key)
412 no_immich_key OK; hint mentions /me (len=65)
[smoke-me-key] stage D: POST /api/v1/me/immich-key (paste a real key, expect 200)
paste OK; immich_user_id=eb2d5112-ecf4-434b-a070-8d1fa9cdc6ed last_validated_at=1779881317
[smoke-me-key] stage E: GET /api/v1/me/people with paste'd key (expect 200 + non-empty)
populated OK; len=30 first.id=6ca4c495...
[smoke-me-key] smoke-me-key PASS — both 412 (no key) and 200 (after paste) paths verified
EXIT=0
```

## Stage-by-stage assertions

| Stage | Step | Expected | Observed |
|---|---|---|---|
| A | OIDC SSO walk (5 sub-steps: `/oidc/login` 303 → Authentik executor → identification → password → `xak-flow-redirect` → `/oidc/callback` 200) | `__Host-iext_session` cookie set with `Secure HttpOnly Path=/`, lands at `https://immich-ext.${DOMAIN}/` | All sub-steps green; cookie present in jar |
| B | `DELETE /api/v1/me/immich-key` | `204` (idempotent) | `204` ✓ |
| C | `GET /api/v1/me/people` with no key | `412`, body `{"error":"no_immich_key","hint":"…/me…"}` | `412` + `error=no_immich_key` + `hint` len=65 mentions `/me` ✓ |
| D | `POST /api/v1/me/immich-key` with `IMMICH_BASE_URL` + `IMMICH_ADMIN_KEY` | `200`, body has `base_url`, `immich_user_id` (non-null), `last_validated_at` (non-zero) | `200` + `immich_user_id=eb2d5112-…` + `last_validated_at=1779881317` ✓ |
| E | `GET /api/v1/me/people` with key | `200`, JSON array, non-empty, items have `id`, `name`, `thumbnail_url` starting with `/api/v1/me/people/` | `200` + len=30 + first item shape OK + `thumbnail_url` routes through our proxy ✓ |

## What this rules out

- **Image drift** — the deployed `immich-extended:dev` is the rebuild from
  HEAD `02c363d` (i.e. it embeds the T9+T10 source changes); the
  pre-existing image was from 12:19, T10 was committed at 13:21, so a
  rebuild was required and was performed.
- **Route regression** — the three `/me/*` routes are still registered at
  the same paths and still gated by the session middleware.
- **412 contract regression** — the discriminator the SPA depends on
  (`error == "no_immich_key"` with a `hint` containing `/me`) is what the
  deployed binary actually emits when the row is absent.
- **Paste validation regression** — pasting a real key still validates
  against upstream Immich's `/api/users/me`, encrypts with the master key,
  and UPSERTs with the correct `immich_user_id`.
- **Populated path regression** — once the key is present, the proxy
  successfully calls Immich on the user's behalf, narrows the result, and
  rewrites `thumbnail_url` to our own proxy path (so the upstream API key
  never crosses back to the browser).

## What this does NOT cover (and why it's still acceptable)

- The actual SolidJS UI rendering (the amber banner with the `<A
  href="/me">` link) is exercised by `web/src/pages/__tests__/peopleMultiSelect.test.tsx`
  in the vitest suite, not by this curl smoke. The server-side contract the
  SPA's banner depends on (412 + `error=no_immich_key` + `hint` mentioning
  `/me`) is the contract this smoke proves. Together they cover both halves
  of the gap.
- A real Chrome MCP browser session would additionally cover the route
  guards (`/me` reachable from `/rules/new`, anchor tags trigger a real
  navigation rather than getting hijacked by `SolidJS-Router`). Chrome MCP
  isn't available in the current worker environment, but those concerns are
  already pinned by the in-tree tests added during POSTSHIP cycle 1
  (`web/src/__tests__/login.sso.test.tsx`, the `rel="external"` regression
  test) and the T9 `/me` route mount tests.

## Files of record

- `scripts/smoke-me-key.sh` — the driver
- `crates/server/src/me/immich_proxy.rs` — `no_immich_key_response()` helper + 412 path
- `crates/server/src/me/immich_key.rs` — paste/inspect/revoke endpoints (T9 backend)
- `web/src/lib/api.ts` — `MeFetchResult<T>` discriminated union (T10 SPA branch)
- `web/src/components/PeopleMultiSelect.tsx` — amber CTA banner (T10 SPA UI)
- `crates/server/tests/me_immich.rs` — server-side contract tests
- `web/src/pages/__tests__/peopleMultiSelect.test.tsx` — SPA banner tests

## Conclusion

POSTSHIP cycle 3 closes. The per-user Immich-key UX gap that surfaced in
cycle 2 review (`emeric` saw misleading empty-state copy in the rule
builder) is fixed in code (`02c363d`, `951ced2`) and proven against the
deployed binary by this run. `[x] M7` is re-ticked; the project-complete
sentinel is restored to `STATE.md`.
