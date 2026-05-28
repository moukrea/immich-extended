# Background whole-library pre-processing index — design

**Status**: DESIGN (POSTSHIP-T27). No code shipped yet.
**Gates**: T28 (schema + indexer task), T29 (rewire matching + album-fill onto the index), T33 (global live log), T36 (per-rule match counts).
**Owner**: worker. The "Open questions" in §9 are flagged for operator review but the LOCKED DECISIONS D1–D6 (`.ralph/TASKS.md`) already resolve the big forks; this doc does not re-open them.

This document is the contract for the background indexer. T28/T29/T33/T36 implementers MUST NOT re-decide the shape; they may only refine details flagged "open" in §9. Authority order: `PRD.md` → `CLAUDE.md` OPERATOR DIRECTIVES (cycle 5) → LOCKED DECISIONS D1–D6 → this doc. Anywhere this doc conflicts with those, they win.

---

## 1. Goal & motivation

Today each rule, on each tick, fetches `POST /api/search/metadata?updatedAfter=<per-rule watermark>` from Immich, evaluates the returned page window, files matches, and advances its own watermark (`crates/server/src/engine_cycle.rs`). Two structural problems fall out of that model:

1. **Backfill is fragile.** The watermark is the only memory of "what have I seen". When a rule's album is bound late (managed album minted on first cycle), or an old photo gets a face tagged after the watermark passed it, the match is never re-filed. T26 patched the album-binding case by resetting the watermark to NULL once; this design removes the watermark from the matching path entirely so the class of bug cannot recur.
2. **Every rule re-fetches the same library.** N rules owned by one user each walk Immich independently. The work is O(rules × library) per cycle and Immich-bound.

The fix (operator directive cycle-5 §2): **index the user's entire library once, in the background, into local SQLite.** Rule matching then becomes a fast local query over pre-computed rows; album-fill re-files *all* current matches, not just new-since-watermark. New/changed assets get indexed incrementally. This naturally fixes the backfill bug (#1 above), removes the per-rule Immich fan-out (#2), and produces the event stream the live log (T33) renders.

**Non-goal for this cycle (D1):** the indexer does NOT run YOLO over the library. YOLO stays lazy + cached (§4.3). The index holds only cheap metadata Immich already returns.

---

## 2. Schema — `asset_index`

One row per `(user_id, asset_id)`. Columns are exactly the D1 set — **no YOLO columns** (D1 overrides T27's original "`yolo_person_count NULL` column" idea; YOLO lives in the existing `asset_yolo_cache`, see §4.3).

```sql
-- migrations/0009_asset_index.sql  (T28)
CREATE TABLE IF NOT EXISTS asset_index (
    user_id      TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    asset_id     TEXT    NOT NULL,
    filename     TEXT    NOT NULL,            -- Immich originalFileName, for the decisions table (T32) + live log (T33)
    updated_at   INTEGER NOT NULL,            -- Immich updatedAt, unix-seconds. The change anchor (D2).
    taken_at     INTEGER,                     -- EXIF dateTimeOriginal ?? fileCreatedAt, unix-seconds. NULL when Immich has neither.
    lat          REAL,                        -- EXIF GPS; NULL when absent
    lng          REAL,
    media_type   TEXT    NOT NULL,            -- 'photo' | 'video' | 'other' (mirrors engine::AssetType)
    person_ids   TEXT    NOT NULL DEFAULT '[]', -- JSON array of Immich person ids (faces) on the asset
    face_count   INTEGER NOT NULL DEFAULT 0,  -- denormalized len(person_ids), for cheap SQL counting (T36)
    indexed_at   INTEGER NOT NULL,            -- when this row was last upserted, unix-seconds
    PRIMARY KEY (user_id, asset_id)
);

-- Matching always scans one user's whole library: this is the hot index.
CREATE INDEX IF NOT EXISTS asset_index_user_idx ON asset_index (user_id);
-- Incremental sweep resume + "newest first" live-log ordering.
CREATE INDEX IF NOT EXISTS asset_index_user_updated_idx ON asset_index (user_id, updated_at DESC);
```

Notes:
- **Timestamps INTEGER unix-seconds** everywhere, matching `asset_decisions`, `rule_runs`, `rules` (the codebase standard; see `0005_engine.sql`).
- **`person_ids` JSON, plus denormalized `face_count`.** D1 lists both. `person_ids` is authoritative (the matching walk needs the actual ids for `Person`/`FaceRecognition` leaves); `face_count = len(person_ids)` is a convenience column so T36 can `SELECT count(*) … WHERE face_count …` without parsing JSON. Keep them in sync on every upsert. Immich's `withPeople:true` returns both named and unnamed detected persons, so `person_ids` already includes unrecognized-but-detected faces — this is the *Immich face* count, distinct from the *YOLO human* count (§4.3).
- **No `CHECK` on `media_type`** — same liberal-text convention as `asset_decisions.decision`; the closed set lives in code (`engine::AssetType`).
- **Per-user FK + cascade**: deleting a user wipes their index. Per-account isolation (PRD §12) holds because every read filters `WHERE user_id = ?` and the indexer only ever writes rows under the key-owner's id.

### 2.1 New plumbing the indexer needs from `immich-client`

`ImmichAsset` (`crates/immich-client/src/lib.rs:212`) currently has no `filename`. T28 must extend `RawAsset`/`ImmichAsset` to capture Immich's `originalFileName` so `asset_index.filename` (and the T32 decisions table) can be populated from the same `POST /api/search/metadata` call already used by `list_assets`. Everything else (`updatedAt`, `exifInfo.dateTimeOriginal`, `latitude`, `longitude`, `people[].id`, `type`) is already parsed.

---

## 3. The background indexer task

A single tokio task per process (NOT per rule), owning the sweep over **all users that have an Immich key on file**. Wired into `main.rs` startup exactly like the scheduler (`crates/server/src/main.rs:143-165`): construct after migrations, `start()` it, hold an `Arc`, `stop()` it on graceful shutdown.

### 3.1 Structure

```
Indexer {
    pool, master_key, data_dir,
    events: ActivityBus,        // §5 live-log ring buffer
    cancel: CancellationToken,
}
Indexer::start(self: Arc<Self>)  -> spawns the sweep loop
Indexer::stop(&self)             -> cancel + join (mirrors Scheduler)
```

The sweep loop is the canonical `tokio::select! { cancelled, sleep(interval) }` shape (copy `engine_scheduler::build_task`) so a paused/shutting-down process never ticks once more. **No `until pgrep … sleep` polling — ever** (harness rule).

### 3.2 One sweep

```
for each user with a row in immich_api_keys:
    decrypt key (reuse engine_cycle::load_key path)             // per-account isolation: per-user client
    build ImmichClient from the user's base_url
    watermark = index_state.last_updated_at for this user (0 if none)
    page through list_assets(api_key, since = watermark, MAX_PAGES_PER_SWEEP):
        for each ImmichAsset:
            upsert asset_index row (ON CONFLICT(user_id,asset_id) DO UPDATE … indexed_at = now)
            emit ActivityEvent::Indexed { user_id, asset_id, filename, … }   // §5
            track max(updated_at) seen
    persist new watermark = max(updated_at) seen this sweep      // monotonic; never goes backward
    if any rows were upserted: enqueue affected rules for re-match (§4.4)
```

### 3.3 Cadence, batching, backpressure, resume (D2)

- **Watermark = max `updatedAt` seen, one per user**, stored in a tiny new table (NOT reusing per-rule watermarks — those belong to a model we're retiring, §6):
  ```sql
  -- part of migrations/0009 (T28)
  CREATE TABLE IF NOT EXISTS asset_index_state (
      user_id         TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      last_updated_at INTEGER NOT NULL DEFAULT 0,  -- max Immich updatedAt indexed, unix-seconds
      last_swept_at   INTEGER                       -- wall-clock of last completed sweep, for the UI
  );
  ```
  Resume-on-restart is automatic: the next sweep reads `last_updated_at` and asks Immich only for `updatedAfter` that. D2's requirement — *a newly-tagged face on an OLD photo must re-index it* — holds because Immich bumps `updatedAt` on face (re)assignment, so the photo re-enters the `updatedAfter` window and re-triggers its rules (§4.4).
- **Batching**: 250 assets/page (Immich cap, already wired). A sweep drains a user's **entire** `updatedAfter` window in one pass, capped only by the `MAX_SEARCH_PAGES` safety ceiling (50k). *(T28 correction — supersedes T27's "8 pages, resume mid-backfill" suggestion.)* The original plan to bound each sweep to ~8 pages and let the watermark resume mid-backfill is **unsound**: Immich's `search/metadata` orders results by `fileCreatedAt`, not by the `updatedAt` watermark key. A mid-window truncation therefore advances the watermark to the max `updatedAt` *seen so far* — frequently the global max, since an old-capture photo that was recently re-tagged sorts early in `fileCreatedAt` order — leaving the unfetched newest-by-capture tail with `updatedAt <= watermark` permanently (observed live: a 2 007-asset library stranded its last 7 at the 2 000-asset cap). Draining the full window each sweep removes the failure mode while keeping the `updatedAt` ingest watermark (so D2 re-tag detection still holds): a fully drained window leaves nothing below the new watermark. `list_assets` stops at the first null `nextPage`, so a caught-up steady-state sweep is still a single short page regardless of the ceiling — the "spread a huge backfill over ticks" worry is moot for a background task that blocks nothing. (Spreading correctly *would* require a `fileCreatedAt` cursor, which sacrifices the simple re-tag detection; not worth it ≤ 50k.)
- **Cadence**: a fixed sweep interval (suggest 120 s; see open Q9.1). During initial backfill, consecutive sweeps walk forward quickly because each advances the watermark; once caught up, a sweep returns near-zero new rows and is cheap (one `search/metadata` call returning an empty/short page per user).
- **Backpressure**: sweeps are sequential per process and bounded per sweep, so the indexer cannot outrun SQLite or Immich. The upserts run in modest transactions (batch a page per tx). The indexer and the per-rule scheduler share the pool; SQLite serializes writes, and both do short transactions, so contention is a non-issue at this scale (§7).

### 3.4 Cost of the empty steady state

Once the library is fully indexed, each sweep is one `POST /api/search/metadata?updatedAfter=<recent>` per user returning 0 rows in the common case — cheap and Immich-friendly. Real new uploads / re-tags show up within one sweep interval.

---

## 4. Rule matching against the index (T29)

### 4.1 The key simplification — reuse the existing tree walker

The engine already has a complete, tested asset evaluator: `engine::predicate::evaluate_expr(&MatchExpr, &AssetSnapshot)` with lazy-YOLO deferral via `DecisionReason::YoloUnimplemented`, and `engine_cycle::decide_with_optional_yolo` wrapping it. **Matching against the index reuses both unchanged.** The only thing that changes is the *source* of the `AssetSnapshot`: instead of mapping a freshly-fetched `ImmichAsset` (`snapshot_from_immich`), we map an `asset_index` row.

```
asset_index row → AssetSnapshot {
    id: asset_id,
    asset_type: parse(media_type),
    taken_at: taken_at.map(epoch_to_utc),
    gps: (lat, lng) when both present,
    face_person_ids: parse_json(person_ids),
    yolo_person_count: None,        // filled lazily on YoloUnimplemented, exactly as today
}
```

This is the single most important design choice: it preserves the back-compat guarantee for free. The Appendix-A YAMLs and the two production rules evaluate through the *same* code path they do now, so their decisions are bit-for-bit identical — only the row source moved from "Immich page" to "local index".

### 4.2 Match pass (per rule)

```
load rule → MatchExpr (serde_json from parsed_predicates, as today)
rows = SELECT * FROM asset_index WHERE user_id = <rule owner>          // whole library, not a watermark window
for each row:
    snapshot = row → AssetSnapshot
    outcome  = decide_with_optional_yolo(match_expr, snapshot)         // lazy YOLO, §4.3
    record asset_decisions (upsert, as today)
    emit ActivityEvent::Decided { rule_id, asset_id, decision, reason }
matched_ids = rows where outcome.matched
→ album-fill diff (§4.5)
```

At ~10k rows this is an in-memory boolean-tree walk over a few MB of metadata — sub-second (§7). We deliberately do **not** translate the arbitrary `MatchExpr` AND/OR/NOT tree (with haversine geo + date + person-set leaves) into SQL: the tree walker already exists, is tested, and SQL push-down would risk diverging from `evaluate_expr`'s exact semantics. SQL pre-filtering is a future optimization (Q9.2), not in scope.

### 4.3 YOLO stays lazy + cached (D1)

Unchanged from today: when `decide_with_optional_yolo` gets `YoloUnimplemented` (every cheaper predicate passed, a YOLO leaf remains), it consults `asset_yolo_cache` (key = `asset_id`, model-version-aware), and on a miss downloads the asset via the owner's key, runs inference, caches the count forever, and re-evaluates. The indexer never populates YOLO. So YOLO cost is paid once per asset that actually reaches a YOLO-gated rule, and never again — exactly the existing `resolve_yolo_count` behavior, now driven from index rows.

### 4.4 What triggers a match pass

Three triggers, all converging on "re-evaluate this rule against the current index":
1. **Indexer upserted rows for a user** → enqueue that user's active rules (the assets they care about may have changed). Coalesce: one pass per rule per sweep even if many rows changed.
2. **Rule created/edited** (CRUD `on_rule_changed`) → enqueue that one rule.
3. **Periodic safety re-match** at the rule's `poll_interval_seconds` (the scheduler stays, but its tick body changes from "fetch-since-watermark + evaluate" to "match against index" — §6).

A small bounded work-queue (or simply: the scheduler tick *is* the match pass, and the indexer sets a per-rule "dirty" flag the next tick honors) keeps this simple. Recommended: **keep the existing per-rule scheduler tick as the match driver**; the indexer just keeps `asset_index` fresh. The tick always evaluates the full indexed set, so "dirty" tracking is an optimization, not a correctness requirement (Q9.3).

### 4.5 Album-fill diff (D3 — respects manual removals)

`album_managed_assets(rule_id, asset_id, state ∈ {added,removed}, changed_at)` already exists (mig 0008, live; T26 populates `added`). The full diff (T29):

```
matched      = matched_ids from §4.2
in_album     = client.get_album_asset_ids(api_key, album_id)            // live Immich truth
removed_set  = SELECT asset_id FROM album_managed_assets
                 WHERE rule_id = ? AND state = 'removed'

# (a) detect operator removals: we filed it, it's gone from the album now → never re-add
for id in (added-state rows) where id ∉ in_album:
    upsert album_managed_assets(rule_id, id, state='removed', now)      # D3 step (c)

# (b) compute adds: matches not already present AND not operator-removed
to_add = matched − in_album − removed_set                               # D3 step (d)

if to_add nonempty:
    client.add_assets_to_album(api_key, album_id, to_add)               # PUT
    upsert album_managed_assets(rule_id, id, state='added', now) for each id  # only after PUT ok (T26 defect-i rule kept)
```

This supersedes T26's watermark-reset stopgap: because the match pass always considers the whole indexed library, "backfill" is just the normal steady-state behavior. Record `added` only after the PUT succeeds (the T26 invariant carries forward). `714dce95` (existing album) and `beba1580` (managed album) both produce correct albums under this path: matched-but-already-present assets are diffed out, operator-removed assets are respected, and previously-missed historical matches are added on the first post-T29 pass.

---

## 5. Live-log hook (T33) — event shape + transport

**Decision: bounded in-memory ring buffer, polled.** Picked over an events table because (a) D4 says `/activity` is polling-based with no SSE — a ring buffer fits polling exactly; (b) no migration, no retention/cleanup job, no write amplification on the hot indexing path; (c) the live log is ephemeral by nature ("what's happening right now"), so durability across restarts is unwanted, not a loss.

### 5.1 Shape

```rust
// crates/server (new module, e.g. activity.rs)
struct ActivityBus { inner: Mutex<VecDeque<ActivityEvent>>, seq: AtomicU64 }  // cap ~500 events, drop oldest

enum ActivityKind {
    Indexed   { filename, person_count, has_gps, taken_at },     // indexer upserted an asset
    Matched   { rule_id, rule_name, asset_id, filename },        // a rule matched an asset
    Skipped   { rule_id, rule_name, asset_id, filename, reason },// decision = skipped (reason slug)
    AlbumAdd  { rule_id, rule_name, album_id, added_count },     // assets filed into an album
    SweepDone { user_id, indexed, took_ms },                     // end of an indexer sweep
}
struct ActivityEvent { seq: u64, at: i64 /*unix*/, user_id: String, kind: ActivityKind }
```

`seq` is a monotonic per-process counter so the poller can request "everything after seq N" and never miss or double-count. Events are **per-user scoped** (`user_id` on every event); the endpoint filters to the session user — no cross-account leakage.

### 5.2 Transport

```
GET /api/v1/me/activity/stream?after=<seq>   (cookie-auth, per-user)
 → { events: [ActivityEvent…], last_seq: u64 }   // only this user's events with seq > after
```

The SPA (`/activity`, T33) polls this every ~2 s, appends, auto-scrolls, caps client-side. `ActivityBus` is held in `AppState`, cloned into the indexer and into `production_tick_fn` so both the indexer and the match passes publish to it. Bounded capacity means memory is O(500 events) regardless of library size.

---

## 6. Migration from the current model

- **Per-rule `last_processed_asset_timestamp` is retired for the matching step.** The match pass (§4.2) ignores it and scans the full index. The column stays in the schema (no migration needed to drop it; harmless), and may be repurposed later as a "last full match pass" marker, but matching no longer reads it. This is what makes the backfill bug structurally impossible: there is no watermark to advance past an unfiled match.
- **`asset_index_state.last_updated_at`** is the only watermark that remains, and it belongs to the indexer (ingest), not to matching. Clean separation (D2).
- **Back-compat for the two production rules** (`beba1580`, `714dce95`): no rule rewrite, no YAML change. On first boot after T28+T29:
  1. The indexer backfills `asset_index` for the owner over the first few sweeps.
  2. Each rule's next tick matches against the (growing) index and fills its album via the §4.5 diff.
  3. `714dce95`'s existing album is diffed (already-present assets excluded → no churn); `beba1580`'s managed album fills with all current matches not operator-removed.
  Until the index finishes its initial backfill, a rule sees a partial library and fills incrementally — never *wrong*, just progressively complete. No operator action required.
- **Ordering of landings**: T28 (index + indexer) can land and run while matching still uses the old watermark path; T29 then flips matching onto the index. Each commit keeps the image building (cycle-5 ABSOLUTE rule). T29 deletes the old fetch-since-watermark code in `cycle_body` (no orphan dead path).

---

## 7. Performance & storage (~10k assets)

- **Row size**: ids are 36-char UUIDs; `person_ids` JSON is a handful of UUIDs for most assets. Estimate ~250–400 bytes/row average. 10k assets ≈ **2.5–4 MB** of table data + ~1–2 MB across the two indexes. Trivial for SQLite. Even 100k assets (~40 MB) is comfortable.
- **Match pass**: `SELECT … WHERE user_id = ?` over 10k rows is a single index range scan (~ms to pull), then an in-memory tree walk per row. `evaluate_expr` is allocation-light boolean logic; 10k walks complete well under a second. Multiple rules each do their own scan — still cheap, and far less work than today's per-rule Immich round-trips.
- **Indexer write load**: steady state is ~0 writes/sweep (nothing changed). Initial backfill is 10k upserts spread over several sweeps in page-sized transactions — bounded and brief.
- **Indexes needed**: `(user_id)` for the match scan, `(user_id, updated_at DESC)` for sweep-resume and live-log "newest first". Declared in §2. No other index warranted at this scale.
- **SQLite mode**: the pool is already the app's single SQLite DB; WAL (if not already on) keeps the indexer's writes from blocking matching reads. Confirm WAL in `common::db::open_pool` during T28 (Q9.4).

---

## 8. Per-account isolation (PRD §12 — preserved)

- The indexer builds a **per-user** `ImmichClient` from each user's decrypted key (same path as `engine_cycle::load_key`); there is no shared client. A user's sweep only ever writes `asset_index` rows under their own `user_id`.
- Every match read filters `WHERE user_id = ?`; a rule only ever scans its owner's index rows, so User A's rule can never see User B's assets (the existing M3-T6 cross-account test's invariant extends naturally — T28/T29 should add an index-scoped variant: User A's match pass never reads User B's `asset_index` rows).
- Activity events carry `user_id` and the stream endpoint filters to the session user.

---

## 9. Open questions (operator review)

1. **Sweep cadence.** Proposed fixed 120 s. Faster (e.g. 60 s) surfaces new uploads sooner at slightly more Immich chatter; slower is gentler. Operator-tunable later, but pick a default. *Recommendation: 120 s.*
2. **SQL pre-filter for huge libraries.** Out of scope now (in-memory walk is fine ≤ ~50k). If a library ever dwarfs that, push cheap leaves (media_type, date, bbox-around-geo, face_count) into the `SELECT` to shrink the in-memory set. Flag only.
3. **Dirty-rule tracking vs. always-full-scan.** Recommended: always full-scan on the tick (simplest, correct). A per-rule "dirty since last match" flag set by the indexer is a pure optimization — defer unless tick cost shows up.
4. **WAL mode.** Confirm/enable SQLite WAL in T28 so indexer writes don't block match reads. Likely already on; verify.
5. **Initial-backfill UX.** During first backfill the index (and thus match counts, T36) is partial. Show an indexer-progress indicator on `/activity` ("indexed 4 200 / ~10 000")? Needs a library-total estimate (Immich `search/metadata` total, or count of seen ids). *Recommendation: show a simple "indexing… N assets so far" until the first empty sweep.*
6. **Deleted-in-Immich assets.** `updatedAfter` never reports deletions, so a deleted asset lingers in `asset_index`. Low harm (album-fill diffs against the live album; a stale index row just yields a match that's already-present-or-removed). A periodic full reconcile (walk all ids, prune missing) is a later nicety. Flag only.

---

## 10. Summary of decisions locked by this doc

| # | Decision |
|---|----------|
| 1 | `asset_index(user_id, asset_id, filename, updated_at, taken_at, lat, lng, media_type, person_ids JSON, face_count, indexed_at)`, PK `(user_id, asset_id)`. No YOLO columns (D1). |
| 2 | `asset_index_state(user_id, last_updated_at, last_swept_at)` — one ingest watermark per user, max Immich `updatedAt` (D2). |
| 3 | One process-wide background indexer tokio task, wired into `main.rs` like the scheduler; bounded pages/sweep; resumes from the ingest watermark. |
| 4 | Matching reuses `evaluate_expr` + `decide_with_optional_yolo` **unchanged**, sourcing `AssetSnapshot` from index rows instead of live Immich. Full-library scan, no per-rule watermark. |
| 5 | YOLO stays lazy + cached in `asset_yolo_cache` (D1); indexer never runs it. |
| 6 | Album-fill = `matched − in_album − removed_set`, recording `added`/`removed` in `album_managed_assets` (D3); record `added` only after a successful PUT (T26 invariant). |
| 7 | Live log = bounded in-memory `ActivityBus` ring buffer, polled via `GET /api/v1/me/activity/stream?after=<seq>`, per-user scoped (D4, no SSE). |
| 8 | Per-rule `last_processed_asset_timestamp` retired from the matching path; ingest watermark is the only remaining watermark. |
| 9 | `immich-client` `ImmichAsset` gains `filename` (from `originalFileName`) in T28. |
