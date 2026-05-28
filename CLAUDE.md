# CLAUDE.md — immich-extended behavioral directives

Read each iteration. Stack + conventions + machine-specific tool access. The PRD is *what*; this is *how*.

## Stack (locked)

- **Language**: Rust (stable, latest)
- **HTTP**: `axum` + `tokio`
- **DB**: SQLite via `sqlx` (offline mode; commit `.sqlx/`)
- **Frontend**: SolidJS + TypeScript + Tailwind, Vite
- **Map**: MapLibre GL JS + OpenStreetMap tiles
- **YOLO**: `ort` crate (ONNX Runtime), CPU; YOLOv8n or YOLOv11n
- **Auth**: `openidconnect` crate for OIDC, `argon2` for local passwords
- **Crypto**: AES-256-GCM via `aes-gcm`
- **Image/video**: `image` crate; `ffmpeg-next` or subprocess
- **Logging**: `tracing` + `tracing-subscriber`
- **Config**: env vars

## Credentials store

Single source of truth: **`~/code/immich-extended/.ralph/creds.env`**. Gitignored (whole `.ralph/` dir is). Chmod 600.

Load it at start of every iteration:
```bash
set -a; source ~/code/immich-extended/.ralph/creds.env; set +a
```

`SUDO_PASSWORD` is pre-seeded. Everything else (`AUTHENTIK_BOOTSTRAP_TOKEN`, `IMMICH_BASE_URL`, `DOMAIN`, etc.) you discover from `~/server/` by grepping broadly (`.md`, `.env`, `.yml`, anywhere), then `cat >> creds.env`. If a credential goes stale, re-grep and overwrite.

OIDC credentials you generate (when configuring Authentik for immich-extended) also go in `creds.env`.

**Never log credential VALUES** outside `creds.env`. Reference by env var name in commits, journal, state files.

## Other tool access

- **`gh`** — already authenticated on this burner.
- **`sudo`** — password in `creds.env`. Invoke as `echo "$SUDO_PASSWORD" | sudo -S <cmd>`.
- **Chrome MCP** — for Authentik admin UI and any flow resisting API automation.
- **`~/server/`** — full read/write. Reuse patterns when deploying.

## Workspace layout

```
immich-extended/
├── Cargo.toml                  (workspace)
├── crates/
│   ├── server/                 (binary)
│   ├── engine/                 (rule engine, predicates, scheduler)
│   ├── immich-client/
│   ├── yolo/
│   └── common/
├── web/                        (SolidJS frontend)
├── migrations/                 (sqlx)
├── Dockerfile
├── docker-compose.yml          (local testing)
├── PRD.md                      (immutable)
├── CLAUDE.md                   (this file)
└── README.md
```

## Conventions

### Rust
- `thiserror` for libraries, `anyhow` in the binary crate.
- No `unwrap`/`expect` (outside tests), `unimplemented!()`, `todo!()`, stub `Ok(())` returns.
- Async everywhere except startup/shutdown.

### SQL
- `sqlx migrate add <name>`. Never edit a committed migration.
- `sqlx::query!` / `query_as!`. Commit `.sqlx/`.

### Frontend
- TypeScript strict, no `any`.
- Tailwind utilities only.
- SolidJS primitives, no extra form library.

### Commits
- Conventional, present tense, no period.
- One commit per task.

### Tests
- Public functions with logic: unit test.
- API endpoints: integration test.
- Predicates: positive + negative.
- **Required before M3 verifies**: cross-account isolation integration test.

## Immich API

- Base URL from env (discovered into `creds.env` as `IMMICH_BASE_URL`).
- Auth header: `x-api-key: <key>`.
- Pagination: ~250 cap on list endpoints — use watermarks.
- Idempotent album add: GET existing, diff, PUT only new.

## YOLO

- Model at `${DATA_DIR}/models/yolo.onnx`. Download from pinned GitHub release on first run.
- One `ort::Session` in `OnceCell`.
- Preprocess: letterbox 640×640, BGR→RGB, normalize.
- Postprocess: NMS 0.5, conf 0.5, class 0 only.
- Cache in `asset_yolo_cache` keyed by `(asset_id, model_version)`.

## Per-account isolation (required tests)

1. User A rule referencing User B's `person_id` → 4xx.
2. User A rule targeting an unwritable album → 4xx.
3. Engine running User A's rule never calls Immich with User B's key.
4. `asset_decisions` queries from User A's session never return User B's rows.

## Deployment (M7)

- Discover stack patterns from `~/server/`. Persist to `creds.env`.
- Create `~/server/immich-extended/docker-compose.yml` matching them.
- Authentik OIDC: API first (token from `creds.env`), Chrome MCP fallback.
- Post-deploy: `/health` returns 200 over HTTPS, real rule + real Immich + real poll → real decision.

## When unsure

Read more context. Try different approaches. Don't mark `[BLOCKED]`. Don't escalate. Don't quit.

---

## OPERATOR DIRECTIVES (2026-05-27, POSTSHIP cycle 4)

The operator has explicitly authorized the scope expansions below. These DO NOT contradict PRD §2 (non-goals — which prohibits things like multi-server federation, third-party photo backends, billing) or §15 (deferred — large mobile/native apps, paid features). Treat them as authoritative additions to the spec for POSTSHIP cycle 4 only. Earlier ABSOLUTE RULES still apply: never modify `PRD.md`, never log credential VALUES anywhere persistent, never spin polling-loop background bash.

1. **YOLO must "just work" out of the box** — the deployed container should not require operators to set `YOLO_MODEL_URL` / `YOLO_MODEL_SHA256`. The maintainer pins the model URL + SHA256 as compile-time constants in `crates/yolo/src/model.rs`. Env vars become OPTIONAL advanced-user overrides, not a hard requirement. The current "Model URL/SHA256 not configured" startup/runtime guard goes away. SHA256 verification of downloaded bytes stays (supply-chain safety); the operator just doesn't supply the value.
2. **Managed Immich albums** — when a rule's target is `{ type: managed, name: "<X>" }`, the engine creates the album in the user's Immich on the first cycle if it doesn't already exist (lookup by name within the user's albums; create if absent), persists the resulting album_id back to the rule, and uses that id for subsequent writes. The schema supports both `target_album_id` (existing) and `managed_album_name` (auto-create) as alternatives.
3. **Block-based rule builder** — the visual builder layout is being redesigned as a **sentence of composable blocks** with AND/OR/NOT operators. Reads like English ("Include media to album [X] when: ( person Paloma AND count=1 ) OR ( person Paloma AND person Emeric AND count>=2 ) MUST EXCLUDE person Manon"). YAML stays the source of truth but the schema gains an expression tree shape. ALL existing predicates remain supported (date, location with map widget, people-must-include/may-include/must-exclude, media type, people-count via YOLO, allow/disallow unrecognized faces). Geo blocks spawn a map picker below the sentence when added. Old flat-schema rules auto-migrate to the new tree on read.
4. **Immich-style UI** — match Immich's actual look (palette, typography, dark-mode-first, sidebar nav, card patterns). Source the spec by inspecting `~/server/immich/` deployed CSS and/or `github.com/immich-app/immich`'s web app. Capture as `docs/design/immich-style-mirror.md` before applying.
5. **Live activity view** — operator wants to see processing as it happens. Per-rule "Recent runs" + "Recent decisions" panel, polling-based (no SSE yet). The `rule_runs` table gets written for the first time (engine inserts at cycle start, updates with finished_at + assets_processed + error on completion).
6. **Rule poll interval is operator-settable** in the UI (was hard-coded default). Sensible bounds: min 60s, max 86400s (1 day). Existing aggressive defaults stay valid in the DB but new rules created via the UI cap at the bounds.
7. **Delete the M7-T5 smoke-test rule** (`3b2b16f1-13f1-4158-8e07-ace225d31c8f`) — it was a deployment verification artifact, not real data. Cascade through `asset_decisions` per the existing FK.

POSTSHIP cycle 4 ABSOLUTE rules:
- **Old YAML import/export still works** after the schema migration. Round-tripping any of the PRD §6 Appendix A examples through the new parser produces equivalent decision behavior.
- **No breaking changes to running rules** — `beba1580` and `714dce95` continue to function or auto-migrate; the user does not have to re-create rules manually.
- **Branch-safe rebuilds** — every commit that lands also keeps the docker image building cleanly; nothing left in a non-buildable intermediate state.


---

## OPERATOR DIRECTIVES (2026-05-28, POSTSHIP cycle 5)

Operator reviewed the deployed cycle-4 build and gave a large batch of feedback. All authorized. These build on cycle-4 directives; same ABSOLUTE RULES apply. Full task specs in TASKS.md under "POSTSHIP cycle 5".

1. **CRITICAL BUG — managed album never gets backfilled.** `beba1580 Paloma (partage Maman)` created its managed album (`e8e8d5e9`) but it's empty, despite 313 `decision=added` rows. Cause: the 313 matches were decided BEFORE the album existed (target_album_id was empty when the cycle ran), recorded as "added" anyway, and the watermark advanced past them — so they never get re-filed once the album appears. Two defects: (i) record `added` only when the Immich PUT actually succeeds; (ii) when an album is created/changed for a rule, re-evaluate the whole library (reset watermark or backfill) so historical matches land in it.

2. **Background whole-library pre-processing** (the big one). Instead of per-cycle Immich fetch + lazy YOLO, immich-extended should index the user's ENTIRE library in the background: per-asset people/faces, YOLO person count, geo, date, media type — stored locally. New assets get indexed incrementally as they're detected. Rule matching then queries the pre-computed index (fast), and album-filling re-files ALL matching assets (not just new-since-watermark). This also naturally fixes bug #1 and powers the live log (#5).

3. **Immich-style account menu + KILL the redundant sign-out buttons.** Currently sign-out appears in the sidebar, the header (with username+email), AND each rule page header — that's dumb. Replace with ONE circular avatar button top-right that opens a popup (mirror Immich, see operator screenshots) containing: Settings link, theme toggle, Sign out. Remove username+email from the header bar and every other stray sign-out.

4. **Merge Overview/global-Activity into the Rules page.** They're near-identical. One page lists rules with status + match count + last run. Remove the "Signed in as <email> (<name>)" line — it's redundant with the account menu.

5. **Global activity = live processing log.** The global activity view should show a live log of the currently-processing asset and what's being retrieved/decided (powered by the background indexer + rule cycles), NOT a duplicate of the rules list.

6. **Per-rule activity rework.** (a) DROP "Recent runs" — it's meaningless to the operator. (b) The view must clearly show WHICH rule it belongs to (rule name in header). (c) Keep "Recent decisions" but: infinite-scroll/lazy-load; the table fixed-height with internal scroll (not whole-page scroll); matched/skipped filter; show the asset FILENAME + a tiny row-height thumbnail that enlarges in a hover popup — not the raw asset UUID.

7. **True drag-and-drop block builder.** The current RuleBuilderV2 is a glorified form, not what was asked. Want: sentence-like draggable blocks, reorder via drag, group blocks into nested AND/OR/NOT groups, visually compose `IF [big block of conditions] [AND/OR] [other block]`. Maps to the same MatchExpr tree. Geo block spawns a map widget. Must be intuitive to a non-technical user.

8. **Show match count per rule** — "N assets currently matched" on the rule list + edit page. Reconcile "matched per decisions" vs "actually in the album".

Cycle-5 ABSOLUTE rules:
- Bug #1 (album backfill) is the highest priority — fix it first (T26), before the architecture work, so the operator's album fills.
- The pre-processing architecture (T27-T29) is designed before implemented (T27 = design doc).
- The drag-drop builder (T34-T35) is designed before implemented (T34 = design doc).
- Don't write the project-complete sentinel until POSTSHIP-T37 (the cycle-5 close-out) verifies everything live.


### Cycle-5 decisions LOCKED (2026-05-28, operator-confirmed)

The three open design forks are now decided; full detail in `.ralph/TASKS.md` "CYCLE 5 — LOCKED DECISIONS (D1–D6)" which OVERRIDES individual task prose:
- **YOLO = lazy + cached** (index cheap metadata for the whole library; YOLO only on-demand, cached). Do NOT proactively YOLO the library.
- **Albums respect manual removals** (track `album_managed_assets`; never re-add an asset the operator pulled out).
- **No operator review gate** — worker designs AND implements straight through. The compensating control is the **mandatory UI self-assessment (D5)**: Chrome-MCP screenshot + critical comparison to the Immich style mirror / wireframe before any UI task is marked done. vitest-green is NOT enough.
- **Drag-drop builder follows the explicit interaction model in D6** — pill-cards, bordered AND/OR/NOT group containers, "Group selected" as the primary grouping mechanism. Do not ship another flat form.

