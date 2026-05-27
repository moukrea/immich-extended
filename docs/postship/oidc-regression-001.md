# OIDC login regression #001 — root cause

- **Date**: 2026-05-27
- **Reporter**: end user (live smoke of deployed `https://immich-ext.${DOMAIN}/login`)
- **Symptom**: Clicking "Sign in with SSO" leaves the URL bar at `https://immich-ext.${DOMAIN}/api/v1/auth/oidc/login` while the page renders the SPA's "Not found / Back home" component. No request reaches Authentik.
- **Diagnosis status**: **Root cause identified.** No code changes in this iteration (per POSTSHIP-T1's exit criteria — diagnosis only). The fix is the trivial one-line change spelled out in §5 below; it belongs to POSTSHIP-T2.

## 1. Why curl saw a 303 but the browser doesn't

The server-side route is healthy. `curl -sI https://immich-ext.${DOMAIN}/api/v1/auth/oidc/login` returns `HTTP/2 303` with a valid `location:` to Authentik's `authorize` endpoint, and `curl -sL` follows the redirect chain through Authentik's flow UI (HTTP 200). The break is entirely client-side: the browser never *sends* the GET to `/api/v1/auth/oidc/login` because SolidJS Router intercepts the anchor click before the browser can navigate.

## 2. Evidence

### 2.1 Deployed bundle has the correct URL (no stale-bundle issue)

```
$ curl -sSL https://immich-ext.${DOMAIN}/ -o /tmp/prod_index.html
$ curl -sSL https://immich-ext.${DOMAIN}/assets/index-BnxWZfGV.js -o /tmp/prod_bundle.js
$ grep -oE '<a [^>]{0,500}>' /tmp/prod_bundle.js | grep oidc
<a href=/api/v1/auth/oidc/login class="block w-full rounded-md bg-slate-900 px-3 py-2 text-sm font-medium text-white text-center hover:bg-slate-800">
```

The deployed anchor matches `web/src/pages/Login.tsx:97-102` byte-for-byte modulo Vite minification. The `href` is correct (`/api/v1/auth/oidc/login`). **The bundle is not stale.**

### 2.2 SolidJS Router intercepts ALL same-origin anchors by default

`@solidjs/router@0.14.10` registers a delegated `document.addEventListener("click", handleAnchorClick)` (see `node_modules/@solidjs/router/dist/data/events.js:106`). The handler skips a click ONLY if any of the following is true:

```js
// events.js:14-38 (paraphrased)
if (evt.defaultPrevented || evt.button !== 0 || metaKey || altKey || ctrlKey || shiftKey) return;
if (!<a> in composedPath) return;
if (a.target) return;                                       // target="_blank" etc.
if (!href && !a.hasAttribute("state")) return;
if (a.hasAttribute("download")) return;
if (rel.includes("external")) return;                        // <-- opt-out
if (url.origin !== window.location.origin) return;          // cross-origin
if (basePath && !url.pathname.startsWith(basePath)) return;
return [a, url];                                            // intercept
```

Otherwise the handler calls `evt.preventDefault()` and `navigateFromRoute(...)` — i.e., `pushState` then SPA route match.

For the SSO button:
- `target` — absent.
- `download` — absent.
- `rel="external"` — **absent.**
- `url.origin === window.location.origin` — yes (same host).
- `basePath` — `/` (no nested router base), so prefix check passes trivially.

→ The router intercepts the click. It `pushState`s `/api/v1/auth/oidc/login` into the URL bar (so the address shows that path) and runs the route matcher. None of `/login`, `/setup`, `/`, `/rules`, `/rules/new`, `/rules/:id`, `/rules/:id/decisions` match — the only remaining route in `web/src/App.tsx:82-92` is `<Route path="*" component={NotFound} />`. That is the exact "Not found / Back home" panel the user sees (`web/src/App.tsx:69-80`).

This explains 100% of the observed symptoms with no further hypothesis needed.

### 2.3 Sanity check — no other internal `/api/...` anchors

```
$ grep -rnE 'href=["'"'"']/api/' web/src
src/pages/Login.tsx:98:              href="/api/v1/auth/oidc/login"
```

Only one. The other internal `<a href="...">` links in the codebase (`/`, `/rules`, `/rules/new`, etc.) ARE intended as SPA routes and remain correct as-is. The `OpenStreetMap` attribution anchor (`MapPicker.tsx:46`) uses `target="_blank"`, which short-circuits the router. The export-rule anchor (`RuleBuilder.tsx:816`) uses a `data:` URL plus a `download` attribute, both of which short-circuit the router.

## 3. Hypotheses ruled out

| Hypothesis | Evidence against |
|---|---|
| Stale production bundle | The deployed `index-BnxWZfGV.js` contains the exact `href="/api/v1/auth/oidc/login"` from current `Login.tsx`. |
| Authentik provider/app misconfig | Server-side `curl` already reaches Authentik's `/if/flow/default-authentication-flow/` with HTTP 200. The browser never *gets* to Authentik to begin with. |
| Callback handler errors | Callback handler is never reached — the initial GET to `/api/v1/auth/oidc/login` is intercepted before any request is sent. |
| Session-cookie attribute mismatch | Same as above — no callback, no cookie. |
| Scope mismatch / signing alg / redirect_uris | Same as above. (The observed `scope=openid+openid+email+profile` duplicate is cosmetic — see §6 below.) |

## 4. Why iteration 7 ("M7-T6 COMPLETE") missed this

The M7-T5 smoke (commit run, JOURNAL entry of 2026-05-27T02:34Z) validated the OIDC chain via curl and via a server-driven walkthrough that touched Authentik's password stage *directly*, never via the SPA's anchor. The integration tests in `crates/server/tests/oidc.rs` drive the OIDC callback handler with a wiremock IdP but also never exercise the *browser's* `<a>` click. The full browser flow was the gap in test coverage. POSTSHIP-T3 closes that gap.

## 5. Fix (for POSTSHIP-T2 — do NOT apply in this iteration)

Single-line change in `web/src/pages/Login.tsx:97-101`:

```diff
-            <a
-              href="/api/v1/auth/oidc/login"
-              class="block w-full rounded-md bg-slate-900 px-3 py-2 text-sm font-medium text-white text-center hover:bg-slate-800"
-            >
+            <a
+              href="/api/v1/auth/oidc/login"
+              rel="external"
+              class="block w-full rounded-md bg-slate-900 px-3 py-2 text-sm font-medium text-white text-center hover:bg-slate-800"
+            >
               Sign in with SSO
             </a>
```

Equally valid alternatives:
- `target="_self"` (also short-circuits the router via the `target` check, but adds a slight semantic mismatch — `_self` is the default).
- Replace the anchor with a `<button>` whose `onClick` does `window.location.assign("/api/v1/auth/oidc/login")`.

Recommendation: `rel="external"` — it's the documented Solid Router opt-out and reads as intent (this link genuinely IS external from the SPA's routing perspective).

After applying:
- Rebuild Docker image, `docker compose up -d --force-recreate immich-extended` on `~/server/`.
- Re-curl `/assets/index-*.js` for the new bundle hash, confirm the deployed anchor now reads `<a href=/api/v1/auth/oidc/login rel=external class=…>`.
- Live-click test in a browser → must land at Authentik login UI, not the SPA NotFound.

## 6. Side-note — duplicate `openid` scope

The pre-investigation noted `scope=openid+openid+email+profile` on the authorize URL. This is cosmetic, not load-bearing:
- The OpenID Connect spec defines `scope` as a space-separated set; servers MUST de-duplicate.
- `crates/server/src/auth/oidc.rs` passes `["openid","email","profile"]` to `openidconnect`; the crate then unconditionally adds another `"openid"` because the spec requires `openid` to be present. Our explicit add is the duplication source.

Not the regression cause. Worth a separate one-line cleanup later, but not in POSTSHIP-T2's blast radius.

## 7. Hand-off to POSTSHIP-T2

- Root cause is **not server-side, not Authentik-side, and not a stale bundle** — it is a missing `rel="external"` on the SSO anchor in `web/src/pages/Login.tsx:97-101`.
- POSTSHIP-T2 applies the one-line fix, rebuilds the Vite bundle, rebuilds the Docker image, redeploys, and verifies the deployed bundle now has the `rel=external` attribute via curl.
- POSTSHIP-T3 adds a vitest that mounts `<Login oidcEnabled={() => true}/>` inside a real `<Router>` and asserts a click on the SSO anchor performs a full browser navigation (e.g., by spying on `window.location.assign` / `pushState`, or by checking the anchor has `rel="external"`).
- POSTSHIP-T4 re-verifies the live flow.
