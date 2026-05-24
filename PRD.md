# immich-extended — Product Requirements Document

**Status:** Draft v0.1
**Owner:** Emeric Commenge
**Last updated:** 2026-05-24

---

## 1. Problem

[Immich](https://immich.app) is an excellent self-hosted photo platform with first-class face recognition, geolocation indexing, and per-user libraries. However, its built-in album automation — Smart Albums — is anemic: filters are limited to date, media type, and a few flat metadata fields. There is no way to express compound predicates that combine **faces present, faces absent, geographic zone, time window, and unidentifiable humans** into a single rule that maintains an album automatically.

Real-world photo curation thinks in those compound terms:

- *"Photos of my daughter where I'm also present, and no one else is identifiable — share with grandma."*
- *"All photos taken in this geographic area, during this trip's date range — share with my partner."*
- *"Photos of the kids without any unknown adult faces — auto-curate into the family album."*

There is also a `immich-face-to-album` Python CLI on PyPI that addresses a narrow slice of this (one face → one album), but it is limited by the underlying Immich search endpoint's 1000-result cap, has no compound predicate support, no UI, no per-rule lifecycle, and no privacy-sensitive logic (e.g., excluding photos where unidentified humans are present).

**immich-extended fills this gap** by providing a rule engine that sits next to Immich, polls for new assets, evaluates user-defined compound predicates, and synchronizes target albums accordingly.

---

## 2. Goals & Non-goals

### Goals

- Allow users to define **declarative rules** that auto-populate Immich albums based on compound predicates (faces, geo, time, media type).
- Support a **safe-by-default** mode for sensitive sharing rules: skip any asset containing an unidentified human (detected via YOLO person counter).
- Provide a **WebUI** suitable for casual users (visual rule builder, map-based geo picker) and an **advanced YAML editor** for power users.
- Preserve Immich's existing **per-account isolation**: a user's rules operate only on their own Immich library and their own face registry.
- Be deployable as a **single container** behind a reverse proxy (Traefik), integrating with existing self-hosted stacks.
- Support **OIDC authentication** (Authentik, Keycloak, etc.) as well as **local accounts**.

### Non-goals

- **Not a replacement for Immich.** It is a sidecar that depends on a working Immich instance.
- **Not a face recognition system.** Face detection and recognition stay in Immich; immich-extended consumes Immich's results via API.
- **Not multi-tenant SaaS.** Single-deployment per family/household.
- **Not a photo editor, viewer, or backup tool.**
- **No NLP, no place-name geocoding, no AI-driven rule suggestions.** Rules are explicit and deterministic (see Design Principles).
- **No automatic backfill beyond what the predicate matches.** If a user wants to include past photos, they set the date predicate to include the past.
- **No dry-run / preview mode in v1.** Rules go live immediately on creation; users can pause or archive if needed.

---

## 3. Target Users & Use Cases

### Primary persona — Power user homelab owner (Emeric)

Self-hosts Immich among many services. Comfortable with YAML, Docker, OIDC. Wants compound rules and full control. Already has Authentik gating his services.

### Secondary persona — Casual family member (Manon)

Logs into Immich on phone. Doesn't care about YAML, but wants to create rules like "all photos from our Paris trip last week go in the shared album."

### Core use cases

| # | Use case | Predicates involved |
|---|---|---|
| UC-1 | Auto-share photos of child with grandparent, only when partner or unknown adults are absent | people (must_include, must_exclude_other_identifiable, no_unidentified_humans) |
| UC-2 | Build a trip album from photos taken in a geographic zone during a date window | date, location, optional people |
| UC-3 | Curate photos of just one person without any cohabitants identified | people (must_include, must_exclude) |
| UC-4 | Build an "everyone together" album where multiple specific people are present | people (must_include = [A, B, C]) |
| UC-5 | Recurring rule: every December, photos at a specific location | date (recurring, deferred), location |

---

## 4. Design Principles

These principles act as guardrails against scope creep and over-engineering. They are non-negotiable.

### P1 — No magic resolution
Rules are expressed in explicit primitives: coordinates, dates, Immich IDs. No place-name geocoding, no NLP, no inference of user intent. If a user wants a zone, they draw it on a map. If a photo lacks metadata required to match a predicate, it does not match. Full stop.

### P2 — Predicate-driven, not procedural
The engine evaluates a set of predicates against each asset and returns a boolean. No loops, no exceptions, no per-rule custom code paths. Adding a new predicate type means adding one handler, not modifying the engine.

### P3 — Per-account isolation is sacred
A rule owned by user A operates only on user A's Immich library and uses user A's face IDs. Cross-account sharing happens via Immich's native shared albums — immich-extended writes into a target album that the user has write access to.

### P4 — Safe-by-default for sensitive predicates
The `no_unidentified_humans` predicate, when enabled, blocks any asset where YOLO detects more humans than Immich detected faces. False negatives (skipping a valid photo) are preferred over false positives (leaking a private photo).

### P5 — KISS, single-tenant, single-binary
One Rust binary, one SQLite database, one container. No Kubernetes manifests, no orchestration, no microservices. Deployed alongside Immich, not inside it.

### P6 — Failure is loud
Polling errors, predicate evaluation errors, API failures are logged per rule and visible in the WebUI. Silent skips are forbidden — every decision (add or skip) is recorded with a reason in the `asset_decisions` table.

---

## 5. Architecture

### High level

```
                ┌───────────────────────────────────┐
                │  immich-extended (single binary)  │
                │                                   │
                │  ┌─────────────────────────────┐  │
                │  │  Web UI (SolidJS, Tailwind) │  │
                │  └─────────────────────────────┘  │
                │  ┌─────────────────────────────┐  │
                │  │  HTTP API (Axum)            │  │
                │  └─────────────────────────────┘  │
                │  ┌─────────────────────────────┐  │
                │  │  Auth (OIDC + local)        │  │
                │  └─────────────────────────────┘  │
                │  ┌─────────────────────────────┐  │
                │  │  Rule Engine                │  │
                │  │  - YAML parser & validator  │  │
                │  │  - Predicate evaluators     │  │
                │  │  - Scheduler (per-rule)     │  │
                │  └─────────────────────────────┘  │
                │  ┌─────────────────────────────┐  │
                │  │  Immich Client (per user)   │  │
                │  └─────────────────────────────┘  │
                │  ┌─────────────────────────────┐  │
                │  │  YOLO Inference (ort crate) │  │
                │  └─────────────────────────────┘  │
                │  ┌─────────────────────────────┐  │
                │  │  SQLite                     │  │
                │  │  (rules, state, audit, keys)│  │
                │  └─────────────────────────────┘  │
                └─────────────────────────────────┬─┘
                                                  │ HTTPS
                                                  ▼
                ┌───────────────────────────────────┐
                │       Immich server               │
                │  (already deployed in stack)      │
                └───────────────────────────────────┘
```

### Stack

| Layer | Choice | Rationale |
|---|---|---|
| Language | Rust | Consistent with Emeric's other tooling (cairn, snag, jaunt, Mantafin); fast, single-binary, type-safe |
| HTTP server | Axum | Modern, ergonomic, integrates well with `tokio` |
| DB | SQLite (via `sqlx`) | Embedded, zero ops, sufficient for single-tenant scale |
| Frontend | SolidJS + Tailwind | Consistent with Rene Deck; lightweight, fast |
| Map (geo picker) | MapLibre GL + OSM tiles | Open, no API key, sufficient quality |
| YOLO runtime | `ort` (ONNX Runtime bindings) | Pure Rust integration, CPU-only sufficient |
| YOLO model | YOLOv8n or YOLOv11n (ONNX) | Small (~6 MB), fast on CPU, person class native |
| Auth | `openidconnect` crate + local Argon2 | Trait-based `AuthProvider` for both flows |
| Config | TOML or env vars | Standard for Rust |

### Polling model

A per-rule scheduled task runs every N minutes (default 5, configurable per rule). For each rule:

1. Fetch the asset deltas since the rule's `last_processed_asset_timestamp` from Immich.
2. Pre-filter via Immich's metadata search (date range, person IDs) to reduce candidate set.
3. For each candidate asset, evaluate all predicates locally.
4. If matched, call Immich `PUT /api/albums/{id}/assets` to add (idempotent — check `albumAssetIds` first).
5. Record the decision (added or skipped, with reason) in `asset_decisions`.
6. Update `last_processed_asset_timestamp` to the latest asset's `updatedAt`.

---

## 6. Rule DSL

Rules are stored as YAML, serialized into the database, and rendered/edited via the WebUI's visual builder. Advanced users can edit YAML directly.

### Schema

```yaml
# Identifier (auto-generated if absent)
id: paris-voyage-juillet-2024

# Human-readable name
name: "Voyage Paris — juillet 2024"

# Owner: filled by server from session, not user-editable in YAML
# owner_user_id: emeric

# Where matched assets are sent
target_album:
  type: existing                     # or "managed"
  album_id: <immich-album-uuid>      # if type=existing
  # name: "Paloma — Mamie"           # if type=managed
  # shared_with: [mamie@example.com] # if type=managed (optional)

# All predicates ANDed. Missing keys = no filter on that dimension.
match:
  date:
    from: 2024-07-15T00:00:00+02:00
    to:   2024-07-22T23:59:59+02:00

  location:
    center: [48.8566, 2.3522]        # [lat, lng]
    radius_km: 60

  people:
    must_include: [<person-id>]              # ALL of these present
    must_include_any_of: [<person-id>]       # at least one present
    may_include: [<person-id>]               # allowed but not required
    must_exclude: [<person-id>]              # NONE of these present
    must_exclude_other_identifiable: false   # any identified face not in {must_*, may_include} causes skip
    no_unidentified_humans: false            # YOLO person_count must == identified_face_count

  media:
    types: [photo, video]            # default: both

# Lifecycle
status: active                       # active | archived | paused
```

### Predicate semantics

**`date.from` / `date.to`** — Asset's EXIF `dateTimeOriginal` (fallback to file mtime) must fall in `[from, to]` inclusive. Either bound is optional.

**`location.center` + `location.radius_km`** — Asset's EXIF GPS coordinates must be within `radius_km` of `center` (haversine distance). Assets with no GPS data do not match.

**`people.must_include: [A, B]`** — All of A and B must be detected as faces in the asset.

**`people.must_include_any_of: [A, B]`** — At least one of A or B must be detected.

**`people.may_include: [A]`** — Pure permission grant. Has no effect on matching, but interacts with `must_exclude_other_identifiable`.

**`people.must_exclude: [A]`** — A must NOT be in the detected faces.

**`people.must_exclude_other_identifiable: true`** — For every identified face in the asset, the face's `person_id` must be in `must_include ∪ must_include_any_of ∪ may_include`. Any other identified face causes skip.

**`people.no_unidentified_humans: true`** — Run YOLO on the asset; require `yolo_person_count == identified_face_count`. If YOLO sees more humans than Immich identified faces, skip.

**Combination** — All present predicate categories must be satisfied (AND). Within `people`, all sub-rules must be satisfied.

### Validation rules

- `target_album.album_id` must be writable by the rule owner (verified at rule creation via Immich API).
- All `person-id` references must belong to the rule owner's Immich account (no cross-account person IDs).
- `radius_km` must be in `(0, 20000]`.
- `from <= to` if both present.
- At least one predicate must be specified (no empty-match rules).

---

## 7. Predicate Evaluation Pipeline

For a candidate asset, predicates are evaluated in this order (cheap-first, expensive-last) to short-circuit:

1. **media.types** — instant, just check `asset.type`.
2. **date** — instant, compare timestamps.
3. **location** — fast, haversine on two floats.
4. **people (Immich-derived)** — uses face data already in the asset payload from Immich's pre-filtered query.
5. **people.no_unidentified_humans (YOLO)** — only runs if all prior predicates pass. Triggers an asset download (thumbnail or full resolution depending on asset type), YOLO inference, count comparison.

For **videos** with `no_unidentified_humans`:

- Sample 1 frame every 2 seconds (configurable).
- Run YOLO + face-count check on each frame.
- If **any frame** has more YOLO persons than identified faces, skip the entire video.
- Use ffmpeg via `ffmpeg-next` crate or a subprocess call (TBD during M5).

### Caching

- YOLO results per asset are cached in `asset_yolo_cache` (asset_id, yolo_person_count, evaluated_at, model_version) to avoid re-running for the same asset across multiple rules.
- Face data is fetched fresh from Immich on each poll cycle (Immich is the source of truth).

---

## 8. Authentication & Authorization

### Auth providers

Trait-based abstraction `AuthProvider`:

```rust
trait AuthProvider {
    fn authenticate(&self, request: &Request) -> Result<UserId>;
    fn provider_name(&self) -> &str;
}
```

Two implementations:

**Local**
- Username + Argon2id password hash in DB.
- Self-registration disabled by default; admin creates accounts.

**OIDC**
- Configured via env vars: `OIDC_ISSUER_URL`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`, `OIDC_REDIRECT_URL`.
- Auto-provisions a user on first login (email = unique key).
- Compatible with Authentik, Keycloak, Authelia, etc.

Both providers can be enabled simultaneously. Configuration determines which login methods appear on the login page.

### Immich API key flow (Pattern A, MVP)

1. User logs in (local or OIDC).
2. On first login (or if API key is missing/invalid), WebUI prompts user to paste their Immich API key.
3. Key is encrypted with a server-wide AES-256-GCM master key (loaded from env var `MASTER_KEY`) and stored in `users.encrypted_api_key`.
4. All Immich API calls for that user use the decrypted key.

User is responsible for generating the API key in Immich settings. Documented in setup guide.

### Per-account isolation enforcement

- Every Immich API call is made with the rule owner's API key — never an admin key.
- Rule creation validates that all referenced `person_id`s and `album_id`s belong to (or are writable by) the owner.
- The `target_album_id` must pass an `assets:add` permission check against the user's API key before the rule is saved.

### Session management

- After authentication, server issues a session cookie (HttpOnly, Secure, SameSite=Lax).
- Session TTL: 30 days, sliding.
- Logout invalidates the session server-side.

---

## 9. Per-account Isolation Model

The most subtle part of the system, restated explicitly:

### Each Immich account has its own `Person` registry

When Emeric tags "Paloma" in his Immich, that creates a `Person` row in Immich's DB owned by Emeric, with an embedding computed from Emeric's photos. When Manon tags "Paloma" in her Immich, that creates a **different** `Person` row owned by Manon. These are not linked. They reference the same human but are two separate database entities with two separate face embeddings.

### Implication for rules

A rule owned by Emeric uses Emeric's `person_id`s and operates on Emeric's assets. It cannot see Manon's photos and cannot use Manon's person IDs.

### Implication for sharing

For a "shared album of Paloma" use case, **two separate but parallel rules are required**: one owned by Emeric using his Paloma `person_id`, one owned by Manon using her Paloma `person_id`. Both rules target the **same shared Immich album** (which both users have write access to, as Immich collaborative albums).

This is acceptable friction for v1. A future enhancement ("co-signed rules") is listed in Open Questions but is deferred.

---

## 10. Data Model

SQLite schema, simplified:

```sql
-- Users (local + OIDC)
CREATE TABLE users (
  id              TEXT PRIMARY KEY,          -- UUID
  auth_provider   TEXT NOT NULL,             -- 'local' | 'oidc'
  external_id     TEXT,                      -- OIDC subject (NULL for local)
  username        TEXT UNIQUE,               -- local username (NULL for OIDC)
  password_hash   TEXT,                      -- Argon2id (NULL for OIDC)
  email           TEXT NOT NULL UNIQUE,
  display_name    TEXT,
  immich_user_id  TEXT,                      -- discovered via Immich /users/me
  encrypted_api_key BLOB,                    -- AES-256-GCM encrypted
  is_admin        BOOLEAN NOT NULL DEFAULT 0,
  created_at      DATETIME NOT NULL,
  last_login_at   DATETIME
);

-- Rules
CREATE TABLE rules (
  id              TEXT PRIMARY KEY,          -- UUID or slug
  owner_user_id   TEXT NOT NULL REFERENCES users(id),
  name            TEXT NOT NULL,
  yaml_source     TEXT NOT NULL,             -- canonical YAML, source of truth
  parsed_predicates TEXT NOT NULL,           -- JSON cache for fast eval
  target_album_id TEXT NOT NULL,
  target_album_strategy TEXT NOT NULL,       -- 'existing' | 'managed'
  status          TEXT NOT NULL,             -- 'active' | 'archived' | 'paused'
  poll_interval_seconds INTEGER NOT NULL DEFAULT 300,
  last_run_at     DATETIME,
  last_processed_asset_timestamp DATETIME,   -- watermark
  created_at      DATETIME NOT NULL,
  updated_at      DATETIME NOT NULL
);

-- Audit: every decision the engine made about every asset, per rule
CREATE TABLE asset_decisions (
  rule_id         TEXT NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
  asset_id        TEXT NOT NULL,
  decision        TEXT NOT NULL,             -- 'added' | 'skipped'
  reason          TEXT NOT NULL,             -- 'matched' | 'date_out_of_range' | 'face_missing' | 'unidentified_human' | ...
  decided_at      DATETIME NOT NULL,
  PRIMARY KEY (rule_id, asset_id)
);

-- Run history
CREATE TABLE rule_runs (
  id              TEXT PRIMARY KEY,
  rule_id         TEXT NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
  started_at      DATETIME NOT NULL,
  finished_at     DATETIME,
  assets_evaluated INTEGER NOT NULL DEFAULT 0,
  assets_added    INTEGER NOT NULL DEFAULT 0,
  assets_skipped  INTEGER NOT NULL DEFAULT 0,
  error_message   TEXT
);

-- YOLO inference cache (across rules)
CREATE TABLE asset_yolo_cache (
  asset_id        TEXT PRIMARY KEY,
  person_count    INTEGER NOT NULL,
  model_version   TEXT NOT NULL,
  evaluated_at    DATETIME NOT NULL
);
```

### Retention

- `asset_decisions` is kept indefinitely (small rows, valuable for debugging).
- `rule_runs` is pruned after 90 days (configurable).
- `asset_yolo_cache` is kept indefinitely; invalidated only on model version change.

---

## 11. WebUI

### Pages

| Route | Purpose |
|---|---|
| `/login` | Login page (local form + OIDC button) |
| `/setup` | First-time: paste Immich API key, verify connection |
| `/rules` | List of rules (Active / Upcoming / Past) |
| `/rules/new` | Visual rule builder |
| `/rules/:id` | View, edit, pause, archive, see run history & decisions |
| `/people` | List of Immich-discovered persons for the logged-in user (read-only) |
| `/settings` | Profile, API key management, OIDC info, logout |

### Visual rule builder

Mobile-friendly, single-column form:

1. **Name** — text input.
2. **Target album** — dropdown of writable Immich albums + "Create new managed album" option.
3. **Date predicate** — toggle to enable; from/to date pickers.
4. **Location predicate** — toggle to enable; embedded MapLibre map. User taps to set center, slider for radius. Visual circle rendered.
5. **People predicate** — toggle to enable; multi-select for each sub-rule (must_include, must_exclude, may_include). Multi-selects show persons with their thumbnail (fetched from Immich). Two toggles for `must_exclude_other_identifiable` and `no_unidentified_humans`.
6. **Media types** — checkboxes.
7. **Advanced (YAML)** — collapsible panel showing the live YAML representation, editable directly.
8. **Save** button.

### Rule detail page

- Current status (active / paused / archived) with toggle.
- Last run summary (timestamp, counts).
- Decisions table (paginated): asset thumbnail, decision, reason, timestamp.
- Edit button (returns to builder pre-filled).
- Delete button (with confirmation).

---

## 12. YOLO Integration

### Model

- **YOLOv8n** or **YOLOv11n**, exported to ONNX (FP16 or FP32).
- Model file shipped via download-on-first-run from a GitHub release (avoids 50 MB binary).
- Stored in `$DATA_DIR/models/yolo.onnx`.

### Runtime

- `ort` crate (ONNX Runtime bindings) with CPU execution provider.
- Single global session, reused across requests.
- Input: image as RGB tensor (resized to model input, typically 640×640).
- Output: detection list filtered to `class_id == 0` (person), `confidence >= 0.5`.
- Result: `person_count = len(detections after NMS)`.

### Asset retrieval

- Use Immich's `/api/assets/{id}/thumbnail?size=preview` for images (faster than full resolution, sufficient for person detection).
- For videos, download the file (or stream via Immich's `/api/assets/{id}/video/playback`) and extract frames via ffmpeg subprocess.

### Performance budget

- A single image YOLO pass on CPU: ~100–300 ms on modern hardware.
- A 2-minute video at 1 frame / 2 sec = 60 frames × 200 ms = 12 seconds.
- Acceptable for a polling-based system with non-interactive evaluation.

---

## 13. Deployment

### Container

Single multi-stage Docker image (~30 MB binary + ~6 MB model = ~50 MB total).

### Compose snippet

```yaml
services:
  immich-extended:
    image: ghcr.io/<emeric>/immich-extended:latest
    container_name: immich-extended
    restart: unless-stopped
    networks:
      - traefik
      - internal             # same internal net as Immich for direct API
    volumes:
      - immich_extended_data:/data
    environment:
      - DATA_DIR=/data
      - MASTER_KEY=${IMMICH_EXTENDED_MASTER_KEY}
      - IMMICH_BASE_URL=http://immich-server:2283
      - OIDC_ISSUER_URL=${OIDC_ISSUER_URL}
      - OIDC_CLIENT_ID=${OIDC_CLIENT_ID}
      - OIDC_CLIENT_SECRET=${OIDC_CLIENT_SECRET}
      - OIDC_REDIRECT_URL=https://immich-ext.${DOMAIN}/auth/callback
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.immich-ext.rule=Host(`immich-ext.${DOMAIN}`)"
      - "traefik.http.routers.immich-ext.entrypoints=websecure"
      - "traefik.http.routers.immich-ext.tls.certresolver=desec"
      - "traefik.http.services.immich-ext.loadbalancer.server.port=8080"
```

### Configuration

All configuration via environment variables. No config file required for normal operation. CLI flags supported for one-off ops (`--migrate-db`, `--create-admin`).

---

## 14. Milestones

### M0 — Skeleton (1 week)

- Repo scaffold (Cargo workspace, frontend folder, CI).
- Axum server with `/health` endpoint.
- SQLite schema + `sqlx` migrations.
- Docker build pipeline.
- Basic structured logging (`tracing`).

### M1 — Auth & onboarding (1–2 weeks)

- Local account creation (CLI-bootstrapped admin).
- OIDC provider integration.
- Session management.
- Immich API key paste + encryption + validation.
- `/setup` flow.

### M2 — Rule CRUD (1 week)

- YAML parser + validator.
- Rule CRUD API.
- Basic rules list & rule detail pages in WebUI.
- Manual YAML editor (no visual builder yet).

### M3 — Engine MVP: date + people predicates (1–2 weeks)

- Per-rule scheduler.
- Immich client (people, assets, albums).
- Predicate evaluators for `date`, `media`, `people` (excl. YOLO).
- Decision recording + audit table.
- Album sync logic with idempotence.

### M4 — Geo predicate + map UI (1 week)

- Location predicate evaluator (haversine).
- MapLibre integration in WebUI.
- Map picker in rule builder.

### M5 — YOLO integration (1–2 weeks)

- `ort` crate setup + model download.
- Thumbnail fetcher.
- Person count predicate (`no_unidentified_humans`).
- ffmpeg-based video frame sampler.
- YOLO cache table.

### M6 — Visual rule builder & polish (1–2 weeks)

- Full visual builder with all predicate categories.
- People multi-selectors with thumbnails.
- Rule lifecycle controls (pause/archive/delete).
- Run history & decisions browser.
- Rule export/import (YAML).

### M7 — v1 release (buffer)

- Documentation.
- Setup guide.
- Example rule templates.
- Public repo release.

**Estimated total**: 8–11 weeks of part-time work.

---

## 15. Open Questions (deferred)

These are explicitly **out of scope for v1** but recorded for future consideration:

### Co-signed rules
Allow a rule to declare itself as "co-signed by user X" — both users' Immich scopes are evaluated and combined. Eliminates the need for parallel mirror rules in couple/family setups. Requires careful UX (consent flow, per-user scope mapping).

### Auto-provisioned API keys
Upgrade from manual API key paste (Pattern A) to admin-key-bootstrapped per-user API key generation (Pattern C). Improves UX, requires admin key handling.

### Recurring date predicates
Support "every December" or "every Sunday" via cron-like expressions. Useful for ongoing themed albums.

### Multi-action rules
Beyond "add to album": send notification, apply Immich tag, copy to external storage (Nextcloud), webhook. Each action adds attack surface and complexity.

### Other detectors
Pose, scene, object detection in addition to person. Lets users write rules like "photos of dogs" or "outdoor photos with sunset." Pure scope expansion, not needed for the core thesis.

### Rule sharing & templates marketplace
Allow users to export rules as templates and share them publicly. Could ship with a curated library of "common patterns" (grandparent share, trip album, kids-only, etc.).

### Webhook ingestion (replace polling)
If/when Immich's webhook system matures, switch from polling to event-driven ingestion for lower latency. Optional fallback to polling.

### Mobile app (Tauri)
Wrap the WebUI in a Tauri shell for native mobile experience. Backend already exposes all functionality via HTTP.

---

## 16. Non-trivial Risks

| Risk | Mitigation |
|---|---|
| Immich API breaking changes across versions | Pin Immich version compatibility per immich-extended release; document in README; integration test suite against tagged Immich versions |
| YOLO false negatives leak sensitive photos | `no_unidentified_humans` is opt-in per rule; documentation emphasizes that no system is perfect; users should review the audit log periodically |
| User pastes admin API key by mistake | Validate on save that the key's scope matches the user's Immich account (call `/users/me` and compare) |
| MASTER_KEY loss = all API keys unreadable | Documented in setup guide; users instructed to back up the env var; on key loss, users re-paste their Immich API keys |
| Polling load on large libraries | Per-rule polling intervals; Immich pre-filter query keeps candidate sets small; watermark-based incremental processing |
| Geo coordinates are sometimes wrong (esp. indoor photos) | Document the limitation; users learn to size their radius generously |

---

## Appendix A — Example rules

### "Photos of my daughter where only my wife or I are present"

```yaml
name: "Famille — restreint"
target_album:
  type: managed
  name: "Paloma — Famille proche"
match:
  people:
    must_include: [<paloma-id>]
    may_include: [<manon-id>, <emeric-id>]
    must_exclude_other_identifiable: true
    no_unidentified_humans: true
status: active
```

### "Paris trip — week of July 15"

```yaml
name: "Paris — juillet 2024"
target_album:
  type: existing
  album_id: <album-uuid>
match:
  date:
    from: 2024-07-15T00:00:00+02:00
    to:   2024-07-22T23:59:59+02:00
  location:
    center: [48.8566, 2.3522]
    radius_km: 60
status: active
```

### "All photos of the kids together, no other identifiable humans"

```yaml
name: "Enfants ensemble"
target_album:
  type: managed
  name: "Les enfants"
match:
  people:
    must_include: [<kid1-id>, <kid2-id>]
    must_exclude_other_identifiable: true
status: active
```

---

*End of PRD.*
