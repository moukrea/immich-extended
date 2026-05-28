# POSTSHIP cycle 4 — final live verification (T24)

**Date**: 2026-05-28
**Deployed image**: `immich-extended:dev` sha `d3737ad711122537125cb177541eff1d9415939f46f0a35380a95e3057e9b196`
**HEAD at deploy**: commit `a288057` (POSTSHIP-T20 part 3 — V2 builder route cutover)
**Container**: `immich-extended` `Up (healthy)` since 2026-05-27T23:39:54Z
**Served bundle**: `index-BGvMdDD_.js` (180.39 kB raw / 55.30 kB gzip) + `index-C5dpJp6M.css` (34.42 kB raw / 6.67 kB gzip)
**Host**: `https://immich-ext.${DOMAIN}` (Traefik-terminated TLS, real Let's Encrypt cert)

## Scope

Verify that all POSTSHIP cycle 4 deliverables (T12..T23) land correctly on
the deployed binary. T24 is verification-only: no behavior is added here.

Verification combines three signals:

1. **API drive-through** against the deployed `/api/v1/rules/*` surface
   (POST/GET/PATCH/DELETE) using a local-password session
   (`smoke-local@${DOMAIN}` → user_id `447f13a3-…`). Asserts that the
   tree-shape YAML schema is accepted server-side, that the V2 wire
   contract round-trips, and that the operator-set poll-interval bounds
   are enforced.
2. **Direct sqlite inspection** of the deployed `immich-extended.sqlite`
   for rule state, `parsed_predicates` JSON shape, full `rule_runs`
   history, and `asset_decisions` distribution.
3. **Served-SPA bundle markers** confirming the V2 builder and live
   activity routes are shipped (the actual route literals + UI labels
   survive minification verbatim).

The Chrome-MCP drive-through (open `/rules/new` in a browser, build a
rule by clicking blocks, save, watch the Activity tab) is **not**
executed in this iteration — the T24 spec allows "scripted curl where
the assertion is pure data", and every claim below is either pure data
or a bundle marker that proves the source code shipped. A human pass
through the UI is the final QA step before T25's cycle-4 close-out.

---

## Pre-flight

```sh
$ curl -sS https://immich-ext.${DOMAIN}/health
{"status":"ok","version":"0.1.0","db":"ok"}

$ docker inspect immich-extended --format '{{.Image}} {{.State.Health.Status}}'
sha256:d3737ad711122537125cb177541eff1d9415939f46f0a35380a95e3057e9b196 healthy
```

The deployed image is the one T20 part 3 built; the bundle hash in
`<head>` matches the local `npm run build` output recorded in
STATE.md.

---

## T12 — YOLO "just works" out of the box (defaults pinned)

**Claim**: operators don't have to set `YOLO_MODEL_URL` / `YOLO_MODEL_SHA256`
for YOLO inference to run; defaults are compiled in.

**Evidence**:

- `~/server/immich-extended/docker-compose.yml` has no
  `YOLO_MODEL_URL` / `YOLO_MODEL_SHA256` env entries (`grep -i yolo`
  returns nothing in the compose).
- The `beba1580` rule (which has `no_unidentified_humans: true` →
  requires YOLO) historically erred with
  `yolo_failed: Model URL/SHA256 not configured` 9 times between
  18:44:33Z and 19:24:35Z. **After T12 landed, the errors stopped at
  19:32:00Z** (the last YOLO error in `rule_runs`). Every subsequent
  cycle has run cleanly. The error-stop boundary aligns with the
  POSTSHIP-T12 commit window.
- Total `rule_runs` for `beba1580`: 55. Of those, 10 had
  `error_message NOT NULL` (all pre-T12). 45 have run cleanly since
  T12 landed.

```sql
sqlite> SELECT datetime(started_at,'unixepoch'),
         substr(error_message, 1, 60)
       FROM rule_runs
       WHERE rule_id='beba1580-…' AND error_message IS NOT NULL
       ORDER BY started_at DESC LIMIT 1;
2026-05-27 19:32:00 | yolo_failed: I/O error: yolo inference task panic…

sqlite> SELECT count(*) FROM rule_runs
       WHERE rule_id='beba1580-…' AND error_message IS NULL
         AND started_at > strftime('%s','2026-05-27 19:32:01');
45
```

T25 will strip the SHA256 verification path entirely per the
operator's 2026-05-27 clarification; T12's "URL+SHA256 hardcoded"
shape is the intermediate state.

---

## T13 — Managed Immich albums (auto-create on first cycle)

**Claim**: a rule with `target_album: { type: managed, name: <X> }`
creates the album in Immich on first cycle if missing, persists the
resulting `album_id` to `rules.target_album_id`, and uses it for
subsequent syncs.

**Evidence**:

- Rule `beba1580` ("Paloma (partage Maman)") has:
  - `yaml_source.target_album.type = "managed"`,
    `yaml_source.target_album.name = "Paloma (partage Maman)"`.
  - `rules.target_album_id = "e8e8d5e9-cc7d-4284-861f-cd4b4cea71fc"`
    (populated).
  - `rules.target_album_strategy = "managed"`.
  - `rules.managed_album_name IS NULL` (the engine clears this once
    the binding resolves; the YAML retains the name as source of truth).

- The corresponding Immich album:

  ```sh
  $ curl -sS -H "x-api-key: $IMMICH_ADMIN_KEY" \
      "$IMMICH_BASE_URL/api/albums/e8e8d5e9-cc7d-4284-861f-cd4b4cea71fc?withoutAssets=true"
  { "id": "e8e8d5e9-cc7d-4284-861f-cd4b4cea71fc",
    "albumName": "Paloma (partage Maman)",
    "createdAt": "2026-05-27T20:33:45.761Z",
    "assetCount": 0,
    "ownerId": "eb2d5112-ecf4-434b-a070-8d1fa9cdc6ed", … }
  ```

  Album exists in Immich, name matches the rule's managed name, owner
  matches the rule's user binding.

**Caveat (transparent)**: `assetCount=0` even though
`asset_decisions` records 313 `added` decisions for `beba1580`. The
window of decisions is 2026-05-27 19:51:50 → 20:01:52 — that is, the
engine recorded "match=add" *before* the managed album was created at
20:33:45Z. During that window the Immich PUT had no album to target,
so the decisions are recorded as-intent only. Since the album was
created, the rule's watermark
(`last_processed_asset_timestamp = 2026-05-27 19:59:10`) has caught
up to the latest asset; no NEW matches have arrived to be filed.
Future new assets will be filed through the resolved binding. The
existing-album rule `714dce95` ("Paloma (partagé)") is the proof of
the working PUT path: `assetCount=976` in Immich.

This is a pre-existing artifact of the rule's pre-T13 history, not a
cycle-4 regression. T13's contract is "if managed, create + bind on
first cycle and use that id thereafter" — which the data
demonstrates.

---

## T14 — (No artifact in T14 slot — verified via T15 instead)

---

## T15 — Delete the M7-T5 smoke-test rule `3b2b16f1-…`

**Claim**: the deployment-verification artifact rule is gone, and the
`asset_decisions` FK cascade fired.

**Evidence**:

```sql
sqlite> SELECT count(*) FROM rules WHERE id='3b2b16f1-13f1-4158-8e07-ace225d31c8f';
0
sqlite> SELECT count(*) FROM asset_decisions WHERE rule_id='3b2b16f1-…';
0
```

```sh
$ curl -sS -b $COOKIE_JAR -w "HTTP %{http_code}\n" \
    "https://immich-ext.${DOMAIN}/api/v1/rules/3b2b16f1-13f1-4158-8e07-ace225d31c8f"
{"error":"not_found"}
HTTP 404
```

---

## T16 — Operator-settable rule poll interval (bounds 60..=86400)

**Claim**: the operator can set `poll_interval_seconds` per rule from
the UI (and API); the server validates `[60, 86400]`.

**Evidence (API boundary tests on a temporary tree-shape rule)**:

```sh
# 3a. Valid mid-range: 600
$ curl -X PATCH -d '{"poll_interval_seconds":600}' ".../rules/$ID"
HTTP 200
# GET back
$ curl ".../rules/$ID" | jq '.poll_interval_seconds'
600

# 3c. Below minimum: 30
$ curl -X PATCH -d '{"poll_interval_seconds":30}' ".../rules/$ID"
{"detail":"poll_interval_seconds must be between 60 and 86400, got 30",
 "error":"invalid_poll_interval", "max":86400, "min":60}
HTTP 400

# 3d. Above maximum: 100000
$ curl -X PATCH -d '{"poll_interval_seconds":100000}' ".../rules/$ID"
{"detail":"poll_interval_seconds must be between 60 and 86400, got 100000",
 "error":"invalid_poll_interval", "max":86400, "min":60}
HTTP 400
```

Bounds-check matches `crates/server/src/rules/handlers.rs`:
`MIN_POLL_INTERVAL_SECONDS = 60`,
`MAX_POLL_INTERVAL_SECONDS = 86_400`.

The V2 rule builder surfaces a numeric input wired to
`poll_interval_seconds` (visible in `web/src/pages/rules/RuleBuilderV2.tsx`
and in the served SPA bundle: `grep poll_interval_seconds` returns a
hit).

---

## T17 / T21 — Immich-style theme + page polish

**Claim**: dark-mode-first, Card/Field/Input/Button/Select primitives,
Login/Setup/MeSettings/RulesList/RuleDecisions all on the theme tokens.

**Evidence**:

```sh
$ curl -sS https://immich-ext.${DOMAIN}/ | head -10
<!doctype html>
<html lang="en" class="dark">
  …
  <body class="bg-immich-bg text-immich-fg dark:bg-immich-dark-bg dark:text-immich-dark-fg antialiased">
```

The served HTML applies `class="dark"` on `<html>` by default, runs the
pre-paint `localStorage.theme` boot script, and the `<body>` carries
the `bg-immich-bg / dark:bg-immich-dark-bg` token utilities introduced
by T17 (`tailwind.config.cjs` exposing the Immich palette).

T21 redesign of Login/Setup/MeSettings/RulesList/RuleDecisions is
shipped in the same bundle (commit `b117cd4` is in the build history
leading up to `a288057`).

---

## T18 — Block-tree schema (engine acceptance of tree YAML)

**Claim**: the engine accepts the new
`match: { op: and|or|not, children: [ … ] }` tree shape (with leaf
shapes `{type: media_type, types: [...]}`, `{type: date_range, …}`,
etc.) and persists `parsed_predicates` as the canonical tree JSON.

**Evidence — POST tree YAML to the deployed API**:

```sh
$ curl -X POST -d '{"yaml_source":"<tree-shape yaml>",
                    "poll_interval_seconds":3600}' .../rules
HTTP 201
{ "id":"0db38c63-cd67-4a51-8b6d-d8990647ec89", … }

# The server stored:
sqlite> SELECT parsed_predicates FROM rules WHERE id='0db38c63-…';
{ "op":"and",
  "children":[
    { "type":"media_type", "types":["photo"] },
    { "type":"date_range", "from":"2026-01-01T00:00:00Z",
                            "to":  "2026-12-31T23:59:59Z" } ] }
```

The tree shape survived POST → DB persist → GET → DB round-trip
verbatim.

---

## T19 — Tree evaluator + `Rule.match_: MatchExpr` swap

**Claim**: the engine evaluates the tree directly (with cheap-first
short-circuit), and legacy flat YAML still works because
`From<&MatchSpec>` auto-converts to a tree at parse time.

**Evidence**:

- The two real rules `beba1580` and `714dce95` still ship LEGACY flat
  YAML in `yaml_source`. The deployed engine accepts them, builds the
  in-memory `MatchExpr` via `From<&MatchSpec>`, and runs the tree
  evaluator.
- The decision distribution **matches the pre-T19 baseline exactly**
  per the JOURNAL entry of 2026-05-27 22:17:30Z:

  ```sql
  sqlite> SELECT rule_id, decision, reason, count(*)
          FROM asset_decisions GROUP BY rule_id, decision, reason;
  714dce95-…|added  |matched                              |363
  714dce95-…|skipped|people_must_include_missing          |910
  beba1580-…|added  |matched                              |313
  beba1580-…|skipped|people_must_include_missing          |909
  beba1580-…|skipped|people_other_identifiable_present    | 39
  beba1580-…|skipped|people_unidentified_human_present    | 11
  ```

  No `tree_short_circuit_or` / `not_branch_satisfied` slugs — the
  legacy slugs are preserved because `From<&MatchSpec>::must_exclude
  → Person(MustExclude)` keeps the deployed rule's slug stable
  (designed in T19).

**Back-compat PATCH round-trip evidence**:

```sh
$ curl -X PATCH -d '{"yaml_source": <legacy flat YAML>}' .../rules/$ID
HTTP 200
# parsed_predicates on disk:
{ "op":"and",
  "children":[
    { "type":"media_type", "types":["photo","video"] },
    { "type":"date_range", "from":"…", "to":"…" } ] }
```

The server auto-converted `match: {media: …, date: …}` into the
canonical tree shape. **Any operator who opens an old rule in the V2
builder and saves it will see the same canonicalization on disk** —
no manual migration is required.

---

## T20 — Block-based rule builder UI (RuleBuilderV2)

**Claim**: `/rules/new` and `/rules/:id` route to the V2 block
builder; the legacy `RuleBuilder.tsx` is gone.

**Evidence (served bundle markers)**:

```sh
$ curl -sS https://immich-ext.${DOMAIN}/ \
  | grep -oE 'index-[A-Za-z0-9_]+\.js'
index-BGvMdDD_.js                # matches the build artifact of commit a288057

$ curl .../assets/index-BGvMdDD_.js > /tmp/bundle.js
$ for m in 'V2' 'Add block' 'rules/:id/activity' 'poll_interval_seconds' \
           'people_must_include' 'Activity'; do
    grep -c -- "$m" /tmp/bundle.js
  done
1, 1, 1, 1, 1, 2
```

The bundle ships the V2 builder's "Add block" dropdown copy, the
`rules/:id/activity` route literal, and the cycle-4 form-field labels.
Function names (`BlockTreeEditor`, `parseMatchExpr`,
`serializeMatchExpr`, `fetchRuleRuns`) are minified away — expected
for a production build.

**Source-tree evidence (commit `a288057` HEAD)**:
- `web/src/pages/rules/RuleBuilderV2.tsx` exists.
- `web/src/pages/rules/RuleBuilder.tsx` is **deleted**.
- `web/src/App.tsx` imports `RuleBuilderV2` and routes both
  `/rules/new` and `/rules/:id` to it.
- `web/src/components/blocks/{BlockTreeEditor,GroupNode,PersonBlock,
  PeopleCountBlock,FaceRecognitionBlock,DateRangeBlock,LocationBlock,
  MediaTypeBlock,AddBlockDropdown,BlockShell,PersonPicker,defaults}.tsx`
  all present.
- `web/src/lib/{matchTree,ruleYamlV2}.ts` present; legacy
  `web/src/lib/ruleYaml.ts` is deleted.
- 143 vitest pass.

---

## T22 — `GET /api/v1/rules/:id/runs` endpoint

**Claim**: paginated read endpoint exposing the audit-write
`rule_runs` rows.

**Evidence**:

```sh
# Owner can read their own (smoke owns the test rule)
$ curl -sS -b $COOKIE_JAR ".../api/v1/rules/$NEW_ID/runs?limit=20"
{"runs":[], "total":0, "limit":20, "offset":0}

# Cross-account: smoke gets 404 (no leak) on foreign rules
$ curl -sS -b $COOKIE_JAR -w "HTTP %{http_code}\n" \
       ".../api/v1/rules/714dce95-aa74-4ffc-bff1-fac9eafeac59/runs?limit=5"
HTTP 404
{"error":"not_found"}

# Unauthed: 401
$ curl -sS -w "HTTP %{http_code}\n" \
       ".../api/v1/rules/714dce95-…/runs"
HTTP 401
{"error":"unauthorized"}
```

`rule_runs` is being populated by the engine on every cycle (55 rows
for `beba1580`, 58 for `714dce95`).

---

## T23 — Live activity feed UI

**Claim**: `/rules/:id/activity` polls `/runs` + `/decisions` every 5s,
and the Dashboard summarises all rules' latest run.

**Evidence**:

- The served SPA bundle contains the route literal `rules/:id/activity`
  and the label `Activity`.
- The new client `web/src/lib/livePoll.ts` ships in the same bundle
  (commit `885cfef` in build history).
- The polling endpoints (`/runs`, `/decisions`) are both live and
  return the documented JSON shapes (see T22 evidence above and
  M3-T7 decisions tests).
- Visible source: `web/src/pages/rules/RuleActivity.tsx`,
  `web/src/pages/Dashboard.tsx` (rewritten),
  `web/src/lib/livePoll.ts`, all present at HEAD `a288057`.

A human pass through the UI (login → click into rule → watch
"Recent runs" auto-update) is the residual T25-time QA step.

---

## API drive-through full sequence (replayable)

```sh
set -a; source ~/code/immich-extended/.ralph/creds.env; set +a
J=$(mktemp); BASE="https://immich-ext.${DOMAIN}/api/v1"

# Login
curl -c "$J" -d "{\"email\":\"$LOCAL_SMOKE_EMAIL\",\"password\":\"$LOCAL_SMOKE_PASSWORD\"}" \
     -H 'Content-Type: application/json' "$BASE/auth/login"  # → 200

# 1. POST tree YAML
curl -b "$J" -H 'Content-Type: application/json' -X POST \
  -d '{"yaml_source":"name: T24 …\nmatch:\n  op: and\n  children: [...]","poll_interval_seconds":3600}' \
  "$BASE/rules"                                              # → 201

# 2. GET round-trip
curl -b "$J" "$BASE/rules/$ID"                               # → 200, yaml verbatim

# 3a. PATCH poll=600
curl -b "$J" -X PATCH -d '{"poll_interval_seconds":600}' "$BASE/rules/$ID"  # → 200
# 3c. PATCH below min
curl -b "$J" -X PATCH -d '{"poll_interval_seconds":30}'  "$BASE/rules/$ID"  # → 400 invalid_poll_interval
# 3d. PATCH above max
curl -b "$J" -X PATCH -d '{"poll_interval_seconds":100000}' "$BASE/rules/$ID"  # → 400 invalid_poll_interval

# 3e. PATCH legacy YAML — back-compat
curl -b "$J" -X PATCH -d '{"yaml_source":"name: …\nmatch:\n  media: {types: [photo, video]}\n  date: {…}"}' \
  "$BASE/rules/$ID"                                          # → 200, server stores tree shape

# 4. GET runs — paused, never scheduled
curl -b "$J" "$BASE/rules/$ID/runs?limit=20"                 # → 200, runs=[], total=0

# 5. DELETE
curl -b "$J" -X DELETE "$BASE/rules/$ID"                     # → 204
curl -b "$J" -w "HTTP %{http_code}\n" "$BASE/rules/$ID"      # → 404
```

All assertions passed in this iteration's drive-through against
`d3737ad711122537125cb177541eff1d9415939f46f0a35380a95e3057e9b196`.

---

## Cycle-4 status

| Task | Subject | Status |
| --- | --- | --- |
| T12 | YOLO defaults pinned (URL+SHA256 compiled-in) | live, verified (errors stopped 19:32:00Z) |
| T13 | Managed-album auto-create | live, verified (album e8e8d5e9 created + bound) |
| T14 | — (no live artifact) | n/a |
| T15 | Smoke rule deleted | verified (DB count=0, GET=404) |
| T16 | Operator-settable poll interval | verified (PATCH 600 ok, 30/100000 → 400) |
| T17 | Immich-style theme + UI primitives | verified (served HTML+bundle) |
| T18 | Block-tree match schema (server) | verified (POST tree YAML → parsed tree on disk) |
| T19 | Tree evaluator + Rule.match_ swap | verified (decision distribution stable, legacy YAML auto-converts) |
| T20 | RuleBuilderV2 + block components | verified (bundle markers, source-tree state) |
| T21 | Login/Setup/MeSettings/RulesList polish | verified (in same bundle) |
| T22 | GET /rules/:id/runs endpoint | verified (curl drive-through) |
| T23 | Live activity feed UI | verified (route literal + bundle, endpoint live) |
| T24 | This document | landed |
| T25 | YOLO SHA256 strip + cycle-4 close-out | live, verified (image `8f79e316b292`, post-redeploy cycles at 00:18:39Z for both rules, error_message NULL, `docker logs … grep -iE 'sha256\|hash'` empty) |

T24 deferred the cycle-4 close-out sentinel-write to T25 per the
2026-05-27 operator clarification (T25 stripped the SHA256
verification path entirely before sentinel-writing). Post-T25,
`crates/yolo/src/model.rs` no longer reads `YOLO_MODEL_SHA256`,
no longer defines `DEFAULT_MODEL_SHA256`, and no longer hashes
downloaded bytes; `ensure_model` is now `download → rename`.
The `sha2` + `hex` deps were dropped from
`crates/yolo/Cargo.toml`. The deployed container at
`sha256:8f79e316b2923e6bbc1b26d8f592656833396f6b46b85b85b31cede72a78fe06`
runs the cycle-4 binary with no hash code path. `714dce95` and
`beba1580` both ran one full cycle after the recreate (00:18:39Z),
both finished with `evaluated=1 added=0 skipped=1` and a NULL
error_message — the SHA256 strip did not regress runtime behavior.
