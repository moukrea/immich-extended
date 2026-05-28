# Asset-centric event-driven matching — design

**Status**: DESIGN (POSTSHIP-T38). No code shipped yet.
**Gates**: T39 (matching as two reusable passes), T40 (indexer→matcher wiring), T41 (rule-lifecycle immediate scan), T42 (retire per-rule timers + hourly safety sweep + `poll_interval` removal + `rule_runs` repurpose), T43 (deletion handling), T44 (activity view rework, D5 gate).
**Owner**: worker. The "Open questions" in §11 are flagged for later refinement; the LOCKED DECISIONS L1–L5 + D5 (`.ralph/TASKS.md` "CYCLE 6 — LOCKED DECISIONS") already resolve the big forks and this doc does not re-open them.

This document is the contract for cycle 6. T39–T44 implementers MUST NOT re-decide the shape; they may only refine details flagged "open" in §11. Authority order: `PRD.md` → `CLAUDE.md` OPERATOR DIRECTIVES (cycle 6) → LOCKED DECISIONS L1–L5/D5 → this doc → the cycle-5 `docs/design/preprocessing-index.md` (still authoritative for the index + album side, which this cycle does **not** change). Anywhere this doc conflicts with a higher source, the higher source wins.

---

## 1. Goal & motivation

Cycle 5 landed the right *storage* model — a background indexer (`crates/server/src/indexer.rs`) sweeps every keyed user's whole library into `asset_index` and re-indexes on Immich `updatedAt` change — but matching never moved onto it as a *trigger*. Matching is still **rule-centric polling**: `engine_scheduler.rs` spawns one tokio task per active rule that wakes every `poll_interval_seconds` and runs `engine_cycle::run_one_cycle`. Two consequences the operator hit on the deployed build:

1. **The indexer and the matcher are disconnected.** A sweep can index a freshly-tagged photo, but nothing re-evaluates rules against it until each rule's *next* poll tick happens to fire. The two heartbeats (sweep cadence vs. per-rule poll cadence) are independent, so "new photo appears → it lands in its album" has unpredictable latency.
2. **Activating a rule waits for a tick.** Create/activate/edit a rule and nothing happens until the per-rule timer fires (up to `poll_interval_seconds` later). The backfill the operator expects "now" is deferred.
3. **The Activity view reads as incoherent periodic noise.** Each per-rule tick writes a `rule_runs` row and re-emits the same per-asset verdicts on a fixed cadence, so `/activity` shows bursty "5-second runs" with no asset-centric narrative — "makes no sense" (operator).

The fix (operator directive cycle-6, LOCKED L1–L5): **make matching event-driven.** The indexer sweep is the *only* heartbeat. After each sweep, the asset_ids that sweep touched are evaluated against all of that user's active rules (incremental). Rule create/activate/edit triggers an immediate full-index scan of that one rule (backfill now). A single slow hourly safety sweep re-scans all active rules to catch anything an event missed. Per-rule poll timers retire entirely. The Activity view becomes a library-status header plus a per-asset grouped live narrative.

**Unchanged this cycle (L2):** the album side. `fill_album` + `compute_album_plan` (D3: respect manual removals via `album_managed_assets`) + the record-`added`-only-after-PUT invariant (T26) all stay exactly as they are. *Only the matching trigger changes.* YOLO stays lazy + cached (D1). The `asset_index` schema + the indexer sweep (`preprocessing-index.md`) stay exactly as they are except the one additive field in §8.2.

---

## 2. The model in one picture

```
                       ┌─────────────────────────────────────────────┐
                       │  Indexer (heartbeat, 120 s) — UNCHANGED       │
                       │  sweep_one_user: upsert asset_index rows,     │
                       │  advance ingest watermark, emit Indexed/      │
                       │  SweepDone events                             │
                       └───────────────┬───────────────────────────────┘
            touched asset_ids (this sweep, one user)
                                       │  (T40)
                                       ▼
              ┌──────────────────────────────────────────────┐
   (T41)      │  Matcher::match_assets  — PASS (b)             │
 rule create/ │  the touched ids × ALL that user's active     │
 activate/    │  rules; reuse evaluate_expr + fill_album;      │
 edit ────────┤  per-asset Matched/Skipped/AlbumAdd events;    │
   │ PASS (a) │  NO rule_runs row (incremental, continuous)    │
   │ one rule │  └──────────────────────────────────────────────┘
   ▼ full     │
 ┌────────────┴───────────────┐        ┌──────────────────────────────┐
 │ Matcher::match_rule_full   │        │  Hourly safety task (T42, L4)  │
 │  — PASS (a)                │◄───────┤  every active rule × PASS (a), │
 │  whole asset_index × 1 rule│        │  SummaryOnly verbosity, 1 line │
 │  reuse evaluate_expr +     │        │  per rule, no per-asset spam   │
 │  fill_album; writes a      │        └──────────────────────────────┘
 │  rule_runs audit row       │
 └────────────────────────────┘
```

Both passes funnel through one shared core (`match_candidates_against_rule`, §3.3). Pass (a) feeds it the rule owner's *entire* index; pass (b) feeds it the *touched subset* and loops over every active rule. Nothing else differs.

---

## 3. Matching as two reusable passes (T39)

Today `engine_cycle::run_one_cycle` *is* pass (a) for one rule, but it's entangled with the `rule_runs` insert/finish bookkeeping and only callable per-rule from the scheduler seam. T39 factors the matching out so both passes share it.

### 3.1 The shared core (pure-ish, no run bookkeeping)

Extract the heart of today's `cycle_body` (lines ~225–370 of `engine_cycle.rs`) into a function that takes the rule, the resolved key/client/album, and a **caller-supplied set of candidate assets**:

```rust
/// Evaluate `candidates` against one already-loaded rule, fill its album with
/// the matched subset, record decisions, emit per-asset events. The unit both
/// passes share. Does NOT touch rule_runs (the caller decides whether this
/// match is audit-worthy — §6.3). Returns counts.
async fn match_candidates_against_rule(
    pool, master_key /* for nothing new */, data_dir,
    rule: &LoadedRule,
    key: &ResolvedKey, client: &ImmichClient, album_id: &str,
    match_expr: &MatchExpr,
    candidates: &[IndexedAsset],     // <-- the only axis that differs
    activity: Option<&ActivityBus>,
    verbosity: EventVerbosity,       // Verbose | SummaryOnly  (§6.2)
) -> Result<MatchCounts, CycleError>
```

Body = exactly today's logic, with `candidates` in place of the `load_index_rows(...)` result:
1. `for asset in candidates { decide_with_optional_yolo(...) }` — lazy YOLO unchanged (D1).
2. `matched_ids = candidates that matched`.
3. `fill_album(pool, client, api_key, rule_id, album_id, &matched_ids)` — **unchanged** (§4).
4. Upsert `asset_decisions` for every candidate; emit `Matched`/`Skipped` per asset (Verbose) and `AlbumAdd` when `filled > 0`.
5. `update_last_run(rule_id, now)`.

### 3.2 Pass (a) — full-index scan of ONE rule

```rust
pub async fn match_rule_full(pool, master_key, data_dir, rule_id, activity, verbosity)
    -> Result<RunOutcome, CycleError>
```
= load rule + key + client, `resolve_target_album` (find-or-create managed album, persist id — unchanged), `candidates = load_index_rows(owner)` (the whole library), then `match_candidates_against_rule(...)`. This is exactly today's `run_one_cycle` minus the seam, **plus** it wraps the call in the `insert_run`/`finish_run` bookkeeping (it is audit-worthy — §6.3). Callers: rule lifecycle (T41), hourly safety sweep (T42). `matched_count` (the read-only T36 count) is untouched.

### 3.3 Pass (b) — a touched asset-set against ALL active rules

```rust
pub async fn match_assets(pool, master_key, data_dir, user_id, touched_ids: &[String], activity)
    -> Result<(), MatchError>
```
1. `active = SELECT id FROM rules WHERE owner_user_id = user_id AND status = 'active'`.
2. `candidates = SELECT … FROM asset_index WHERE user_id = ? AND asset_id IN (touched_ids)` (load once, reuse for every rule — same `IndexedAsset` mapping as `load_index_rows`).
3. For each active rule: load rule + key + client, `resolve_target_album`, then `match_candidates_against_rule(rule, …, &candidates, Verbose)` with **no `rule_runs` row** (§6.3).
4. A single rule's failure (rotated key, Immich down) is logged and skipped so it can't abort the others' matching — same resilience contract as `Indexer::sweep_all_users`.

`touched_ids` belong to exactly one user (the indexer sweeps per user — §4), so per-account isolation falls out for free: pass (b) only ever loads that user's index rows and matches them against that user's rules.

### 3.4 Why a PARTIAL matched set is still correct (the crux)

Pass (b) hands `fill_album` only the *touched-and-matched* subset, not the rule's full match set. This is safe because of how `compute_album_plan` (`album_sync.rs`) is written — verified against its source + tests:

- `to_add = matched − in_album − effective_removed`. With `matched` = touched subset, `to_add` is exactly the newly-touched matches not already filed and not operator-removed. The rule's *other* current matches are already in `in_album` from prior passes, so omitting them changes nothing.
- `newly_removed = prior_added − in_album` — computed from the **full** `album_managed_assets` `added` set and the **full** live album, *independent of the `matched` slice*. So operator removals are still detected across the whole album on every incremental pass, even for assets the sweep didn't touch.
- `added_baseline` only re-baselines touched matches; untouched matches keep their existing `added` rows. No row is lost.

The one thing a purely-incremental model can miss: an asset matched in a *prior* pass whose album PUT failed (so it's neither in the album nor recorded `added`) and which is never touched again. The **hourly safety full-scan (L4 / §6.1)** is the explicit backstop for exactly that class. So: pass (b) = fast incremental steady state; pass (a) hourly = eventual full reconcile. Together they are at least as correct as today's every-tick full scan, with far less work.

### 3.5 What we deliberately do NOT change

- We do **not** un-file assets that stop matching. `fill_album` only ever PUTs; it never calls `remove_assets_from_album`. An asset that matched once, got filed, then stopped matching (e.g. a face was untagged) stays in the album. That is the **existing cycle-5 behavior** and L2 says only the trigger changes. Whether "unmatch ⇒ remove from album" is wanted is an open question (§11.5), explicitly out of scope for T39–T45.
- We do **not** translate `MatchExpr` into SQL. The in-memory `evaluate_expr` walk over ≤ a few k touched rows (pass b) or ≤ ~50k full rows (pass a) is sub-second (`preprocessing-index.md` §7). SQL push-down remains a future optimization.

### 3.6 Tests (T39)
- Pass (a): full-scan fills an empty album with all current matches; idempotent re-run adds nothing; respects a recorded `removed`.
- Pass (b): three indexed assets, only two touched & matching → only those two PUT; an untouched-but-operator-removed prior-added asset is still recorded `removed` (proves §3.4 removal-respect on the partial path).
- Cross-account: pass (b) for user A never loads or matches user B's index rows/rules (extends the M3-T6 invariant).
- Lazy YOLO still fires only for assets passing every cheaper predicate of a YOLO rule, and the cache still short-circuits re-inference.

---

## 4. Indexer → matcher wiring (T40)

The indexer stays the heartbeat (L1). The only change: after each *user* sweep, hand the touched ids to pass (b).

### 4.1 Collect touched ids
`sweep_one_user_inner` already iterates the upserted `assets`. Collect their ids into a `Vec<String>` (the assets it actually upserted this sweep) and return them up through `UserSweepSummary` (add a `touched_ids: Vec<String>` field) — or pass them straight to an injected matcher hook before the function returns. Empty sweep ⇒ empty vec ⇒ no match work.

### 4.2 The hook — keep the indexer storage-only via a seam
The indexer must not grow a dependency on the full matcher (it currently doesn't even take `data_dir`, which lazy-YOLO needs). Mirror the scheduler's `RunCycleFn` seam: inject a matcher callback.

```rust
// indexer.rs
pub type OnSweepFn = Arc<
    dyn Fn(String /*user_id*/, Vec<String> /*touched_ids*/) -> Pin<Box<dyn Future<Output=()> + Send>>
        + Send + Sync,
>;
```
`Indexer` holds an `Option<OnSweepFn>` (tests pass `None`; production wires it). After `sweep_one_user_inner` commits + emits `SweepDone`, if the hook is set and `!touched_ids.is_empty()`, `await` it. Production builds the hook in `main.rs` capturing `pool + master_key + data_dir + activity` and calling `engine_cycle::match_assets(...)`. This keeps the indexer free of the engine/YOLO surface and keeps the matcher's `data_dir` out of the indexer struct.

Coalescing: one pass (b) per user per sweep over the whole touched set (not one-per-asset). At steady state most sweeps touch 0 rows ⇒ the hook isn't called. A backfill sweep touches a page ⇒ one pass (b) over that page.

### 4.3 Tests (T40)
wiremock: a user whose Immich reports 3 changed assets ⇒ after one sweep, exactly those 3 are evaluated against the active rules, matching ones land in the album, `Indexed` + `Matched`/`Skipped` events emitted, untouched assets untouched.

---

## 5. Rule lifecycle triggers an immediate full scan (T41)

In `rules/handlers.rs`, `create_rule` / `update_rule` / `delete_rule` currently call `state.scheduler.on_rule_changed(&id)` (fire-and-forget, errors logged + swallowed so a scheduler hiccup never turns a 201 into a 500). Replace that seam:

- **create (active) / update (→active or yaml/predicate change while active)**: trigger pass (a) `match_rule_full(rule_id, Verbose)` for that one rule so its album backfills immediately (L3) — no poll-tick wait.
- **update → paused/archived, or delete**: nothing to schedule (no timers exist anymore). Delete still cascades `asset_decisions` + `album_managed_assets` via the existing rule FK.

### 5.1 Inline vs. spawned
A full-library scan that hits the lazy-YOLO path could download + infer over thousands of assets — too slow to `await` inside the POST handler. So the handler **spawns** the pass (`tokio::spawn`) and returns immediately, preserving today's fire-and-forget ergonomics (the response says "created", the album fills moments later). The trigger is the *request*, not a timer — which is what L3 means by "immediate, not next-tick".

For deterministic tests, T41 asserts the *pass itself* (call `match_rule_full` directly in an integration test and check the album filled + decisions recorded in the same flow) rather than racing the spawned task through HTTP. The HTTP-level test asserts only that creating an active rule *triggers* the pass (e.g. via a test seam / injected matcher that records the call), not the async fill timing.

### 5.2 The seam in `AppState`
`AppState.scheduler: Arc<Scheduler>` is replaced by `AppState.matcher: Arc<Matcher>` (a thin service holding `pool + master_key + data_dir + activity`) exposing:
- `on_rule_activated(rule_id)` → spawns `match_rule_full(.., Verbose)` (used by handlers),
- `match_assets(user_id, touched_ids)` → pass (b) (used by the indexer hook),
- `safety_sweep()` → pass (a) over all active rules, `SummaryOnly` (used by the hourly task).
The pure pass fns live in `engine_cycle`; `Matcher` is the wiring + spawn point. One service, three call sites (handlers, indexer hook, hourly task).

### 5.3 Tests (T41)
Creating an active rule whose predicates match indexed assets fills its target album in the request-driven flow (direct `match_rule_full` call), with `asset_decisions` recorded — no scheduler tick involved.

---

## 6. Retire per-rule timers; add the hourly safety sweep (T42)

### 6.1 Delete `engine_scheduler.rs` per-rule tasks
Remove the per-rule `tokio` task machinery: `Scheduler`'s `running` map, `build_task`, `on_rule_changed`, `start`/`stop` per-rule reconciliation, `interval_for`, the `tick_interval_override` test seam, and `production_tick_fn`'s scheduler coupling. `main.rs` stops constructing/starting/stopping a `Scheduler`. The cycle body (`match_rule_full`) survives — it just isn't driven by a timer anymore.

### 6.2 One global hourly safety sweep (L4)
A single process-wide tokio task (NOT per-rule), canonical `tokio::select! { cancelled, sleep(interval) }` shape (copy the indexer's loop — **never** a `until pgrep … sleep` polling loop, harness rule). Config-driven interval, default 3600 s, via a const + optional env override (`SAFETY_SWEEP_INTERVAL_SECONDS`). Each fire: for every active rule across all users, run pass (a) with `EventVerbosity::SummaryOnly` — suppress per-asset `Matched`/`Skipped` events, emit at most one summary line per rule (reuse/extend `SweepDone`-style summarization, §8). This catches any event the incremental path missed (PUT failure, a sweep the process slept through) without spamming the live log. It writes a `rule_runs` row per rule (audit — §6.3).

`EventVerbosity { Verbose, SummaryOnly }` is the single knob: pass (b) and lifecycle pass (a) use `Verbose` (operator wants to watch); the hourly sweep uses `SummaryOnly`.

### 6.3 `rule_runs` — repurpose, don't drop
`rule_runs` (mig `0005_engine.sql`) was the per-tick audit. Cycle 5 already dropped the "Recent runs" list from the per-rule UI. Decision:
- **Full-scan passes (a)** — rule lifecycle backfill + hourly safety — **write a `rule_runs` row** (`insert_run`/`finish_run`, counters + `error_message` on failure). These are discrete, meaningful, low-frequency events: "this rule was fully reconciled at T; evaluated N, added M". That's a useful coarse audit and is *not* the noise the operator complained about.
- **Incremental pass (b)** — per sweep — **writes NO `rule_runs` row.** Writing one per rule per 120 s sweep is exactly the bursty "5-second runs" noise. Pass (b) surfaces only through per-asset activity events + `rules.last_run_at`.

Net effect: `rule_runs` stops being a high-frequency firehose and becomes an occasional reconcile log. T42 must **grep precisely** whether anything still *renders* `rule_runs` (the cycle-5 `/rules/:id/activity` view dropped the runs table; `RulesList`/`RuleBuilderV2` matches are almost certainly `last_run_at`, not the runs list). If `GET /api/v1/rules/:id/runs` (`list_rule_runs`) + `RunItem`/`RunsResponse` have no remaining consumer, remove the route + handler + frontend client fn; if something still uses it, leave it. Do not drop the *table* (cheap, and the audit rows are now meaningful).

### 6.4 Remove the operator-settable poll interval
With timers gone, `poll_interval_seconds` no longer drives anything. T42:
- Remove the UI field + its client validation from the block builder (`web/src/pages/rules/RuleBuilderV2.tsx` and any builder partial that renders it).
- Remove `validate_poll_interval`, `MIN/MAX/DEFAULT_POLL_INTERVAL_SECONDS` usage from `rules/handlers.rs` create/update (stop reading `poll_interval_seconds` from the request body; drop the field from `CreateRuleRequest`/`UpdateRuleRequest` or accept-and-ignore for back-compat — prefer drop).
- Keep the `rules.poll_interval_seconds` **column** (no migration to drop; existing rows are harmless and a drop is pure churn). It simply goes unread.
Cycle-4 directive #6 (operator-settable interval) is **superseded** by cycle-6 L1 (timers retired) — note this in the commit body.

### 6.5 Tests (T42)
Scheduler module deletion compiles; `main.rs` boots with only the indexer + hourly task + matcher. A unit test for the hourly task's rule selection (all active rules, SummaryOnly). `cargo fmt/clippy/test --workspace` green; docker image still builds (cycle ABSOLUTE rule).

---

## 7. Deletion handling (T43)

`updatedAfter`-based sweeps never report deletions, so an asset deleted in Immich lingers forever in `asset_index` (and its stale `asset_decisions`/`album_managed_assets` rows persist). Low *correctness* harm (album-fill diffs against the live album, so a stale match is already-present-or-removed and never wrongly re-added), but it skews the status header's "indexed N", the match counts, and the live log. T43 prunes them.

### 7.1 Detection — periodic full-id reconcile
`updatedAfter` can't surface deletions, so detection needs a full membership comparison, run at **low cadence** (it's heavier than a steady sweep):
```
reconcile_one_user(user_id):
    live_ids = { every asset id Immich currently returns for this user }   // full listing, no updatedAfter
    indexed_ids = SELECT asset_id FROM asset_index WHERE user_id = ?
    stale = indexed_ids − live_ids
    for id in stale: prune(user_id, id)   // §7.2
```
Run it **inside the indexer** (it already owns the per-user decrypted client + the sweep loop), gated to fire every Nth sweep — default every 30th sweep ≈ hourly at the 120 s cadence (a `sweeps_since_reconcile` counter, or `now − last_reconcile_at ≥ reconcile_interval`). Do **not** run it every sweep. The full listing reuses `ImmichClient::list_assets(api_key, None, MAX_SEARCH_PAGES)`; an id-only Immich listing would be cheaper and is flagged as an optimization (§11.4), not required.

### 7.2 Prune — cascade by hand (no FK path exists)
`asset_decisions` and `album_managed_assets` FK to `rules(id)`, **not** to `asset_index`, so deleting an `asset_index` row cascades nothing. Prune explicitly, scoped to the user via their rules:
```sql
DELETE FROM asset_index        WHERE user_id = ? AND asset_id = ?;
DELETE FROM asset_decisions     WHERE asset_id = ?
    AND rule_id IN (SELECT id FROM rules WHERE owner_user_id = ?);
DELETE FROM album_managed_assets WHERE asset_id = ?
    AND rule_id IN (SELECT id FROM rules WHERE owner_user_id = ?);
```
Batch the stale set in one transaction. **Do NOT touch the Immich album** — the photo is already gone there; issuing a remove would be redundant and could error. Optionally emit a single summary activity line ("pruned N deleted assets"); not a per-asset event.

The composite PKs are `(rule_id, asset_id)`, so the `WHERE asset_id = ?` deletes don't hit an index. At ≤ 50k assets pruning a handful per reconcile is fine; if a bulk deletion ever shows up slow, add `asset_decisions(asset_id)` + `album_managed_assets(asset_id)` indexes in a `migrations/0010` (flagged §11.4, not required for T43).

### 7.3 Tests (T43)
wiremock: user has assets {a,b,c} indexed; Immich now returns only {a,c} ⇒ reconcile prunes b from `asset_index` + its `asset_decisions` + `album_managed_assets`, leaves a/c, and issues no album-remove call.

---

## 8. Activity view rework (T44, L5, D5 gate)

L5: `/activity` = **library-status header** + **per-asset grouped live narrative**. Reshape *consumption* of the existing `ActivityBus`; do not rebuild the bus.

### 8.1 Status header — new endpoint
```
GET /api/v1/me/index/status        (cookie-auth, per-user)
→ {
    indexed:       i64,          // SELECT count(*) FROM asset_index WHERE user_id = ?
    last_swept_at: Option<i64>,  // asset_index_state.last_swept_at
    library_total: Option<i64>,  // best-effort Immich asset count; null if Immich unreachable
    sweeping:      bool          // is a sweep in progress for this user right now?
}
```
Renders: `indexed N / M · last sweep <ago> · idle|indexing`. `library_total` is best-effort from Immich's statistics endpoint — null (not an error) on any missing-key/transport failure, same degradation pattern as `album_asset_count` in `rule_match_count`. `sweeping` is the honest "indexing vs idle" signal: expose it from the indexer via a per-user (or process-wide) `AtomicBool`/`Mutex<HashSet<user>>` flipped around `sweep_one_user_inner`, shared into `AppState`; if that plumbing proves heavy, fall back to deriving `indexing = library_total.map_or(false, |m| indexed < m)`. Endpoint lives at `crates/server/src/me/index_status.rs` next to `me/activity.rs`.

### 8.2 Per-asset grouping — one minimal bus refinement
The `Indexed` event carries `filename` but **no `asset_id`** (`activity.rs:49`), so the client can't correlate an asset's `Indexed` line with its later `Matched`/`Skipped` lines. Add `asset_id: String` to `ActivityKind::Indexed` (and pass it from `sweep_one_user_inner`, which has `asset.id` in scope). This is a single additive field — *not* a bus rebuild — and is wire-compatible (the client reads the new field; the ring buffer, `seq`, transport, per-user filter, `/me/activity/stream` all stay).

With `asset_id` on every per-asset event, the SPA groups the flat stream **client-side**:
- Group `Indexed` + `Matched` + `Skipped` events by `asset_id` (within the rendered window) into one card/line:
  `IMG_1234.jpg  ·  indexed (3 people · GPS)  →  matched "Paloma (partage Maman)"  ·  skipped "Trip 2024" (date out of range)`
  with the existing tiny thumbnail (`/me/assets/:id/thumbnail`, already used) enlarging on hover.
- `AlbumAdd` (rule-level, `added_count`) and `SweepDone` stay as standalone interleaved summary lines ("filed 12 assets into 'Paloma'", "Library sweep — indexed 7"). The per-asset "→ filed to album" is implied by a `matched` against a rule that has an album; we do not add per-asset album events (keeps the bus minimal).
- Ordering: events are `seq`-ordered; group by `asset_id` but key the group's position by its *first* event's `seq` so the narrative reads top-to-bottom in arrival order. Keep the 2 s poll, tail-follow, pause-on-hover, and `MAX_EVENTS` client cap from today's `Activity.tsx`.

### 8.3 Per-rule activity stays a decisions table
`/rules/:id/activity` (`RuleActivity.tsx`) stays the cycle-5 decisions table (filename + thumbnail + matched/skipped filter + infinite scroll), **clearly labeled with the rule name** in the header (cycle-5 T32 already did this — verify it survived). The "Recent runs" list stays dropped (§6.3).

### 8.4 D5 quality gate (mandatory before marking T44 done)
vitest-green is NOT sufficient. After building: `cd web && npm run build`, deploy/preview, open `/activity` in **Chrome MCP**, screenshot light + dark, and critically compare against L5's narrative example and `docs/design/immich-style-mirror.md` (palette/typography/dark-first/card patterns). Iterate if it reads generic or confusing. Save the screenshots to `docs/postship/cycle6-t44-activity-*.png` and a short `…-verify.md`, mirroring the cycle-5 T32/T33 artifacts.

### 8.5 Tests (T44)
Server: `/me/index/status` returns the counts + null `library_total` when Immich is unmocked/unreachable; per-account scoped. Web: vitest for the grouping reducer (a sequence of Indexed+Matched+Skipped for two asset_ids groups into two cards in first-seen order; a lone SweepDone renders as a summary line).

---

## 9. Per-account isolation (PRD §12 — preserved)

- Pass (b) is driven by a per-user sweep; it loads only `WHERE user_id = ?` index rows and matches them only against `WHERE owner_user_id = ? AND status='active'` rules. A per-user `ImmichClient` is built from that user's decrypted key (no shared client) — same as today.
- Pass (a) is scoped to the rule owner (`load_rule` → `owner_user_id`; `load_index_rows(owner)`); the hourly sweep iterates rules and inherits each rule's owner scope.
- `/me/index/status` and `/me/activity/stream` filter to the session user; activity events carry `user_id` and are filtered in `ActivityBus::since`.
- T43 prune is scoped `WHERE owner_user_id = ?` on the rules subquery, so one user's reconcile never deletes another's decisions.
- T39 adds the index-scoped cross-account test (pass (b) for A never reads B's rows) extending the M3-T6 invariant.

---

## 10. Module-by-module change map

| Module | Change |
|---|---|
| `engine_cycle.rs` | Extract `match_candidates_against_rule` (shared core); add `match_rule_full` (pass a, wraps run bookkeeping) + `match_assets` (pass b, all active rules); add `EventVerbosity`. Delete `production_tick_fn`'s scheduler coupling. `matched_count` untouched. (T39) |
| `indexer.rs` | `sweep_one_user_inner` collects touched ids; `Indexer` holds optional `OnSweepFn`, invoked post-sweep; gains the every-Nth-sweep deletion reconcile (T43). No `data_dir` in the struct (matcher hook carries it). (T40, T43) |
| `engine_scheduler.rs` | **Deleted** (per-rule tasks retired). (T42) |
| new `matcher.rs` (or fold into `engine_cycle`) | `Matcher` service: `on_rule_activated`, `match_assets`, `safety_sweep`; held in `AppState`. (T41/T42) |
| new hourly task | One process-wide `select!{cancelled,sleep}` task calling `safety_sweep` (SummaryOnly), config interval default 3600 s. (T42) |
| `rules/handlers.rs` | Replace `scheduler.on_rule_changed` with `matcher.on_rule_activated`; drop `poll_interval_seconds` from requests + `validate_poll_interval` + MIN/MAX/DEFAULT consts. (T41/T42) |
| `activity.rs` | Add `asset_id` to `ActivityKind::Indexed` (only change). (T44) |
| new `me/index_status.rs` | `GET /api/v1/me/index/status`. (T44) |
| `main.rs` | Drop `Scheduler`; build `Matcher` + `OnSweepFn` + hourly task; wire indexer hook; `AppState.scheduler → matcher`. (T40/T41/T42) |
| `web` | `Activity.tsx` status header + per-asset grouping; remove `poll_interval` field from builder; (maybe) drop `/runs` client. (T42/T44) |
| `rule_runs` table | Kept; written by pass (a) only. `/runs` endpoint removed iff unused. (T42) |
| `rules.poll_interval_seconds` column | Kept, unread (no migration). (T42) |
| `migrations/0010` | Only if §7.2/§11.4 indexes prove needed — not required by T43. |

---

## 11. Open questions (later refinement; do not block T39–T45)

1. **Hourly safety cadence.** Default 3600 s. Faster catches missed events sooner at more Immich chatter; slower is gentler. Operator-tunable env. *Recommendation: 3600 s.*
2. **`sweeping` signal source.** AtomicBool/Mutex<set> from the indexer (accurate) vs. derived `indexed < library_total` (zero plumbing, slightly approximate). *Recommendation: the flag if cheap, else derive.*
3. **`library_total` source.** Immich `GET /api/server/statistics` / `/api/assets/statistics` vs. count of distinct ids seen. Best-effort, null on failure either way. *Recommendation: statistics endpoint, degrade to null.*
4. **Deletion-reconcile cost + indexes.** Full-listing every Nth sweep is the simple correct path; an id-only Immich listing + `(asset_id)` indexes (`migrations/0010`) are optimizations if a large-library prune shows up slow. Flag only.
5. **Unmatch ⇒ remove from album?** Today (and after this cycle) an asset that stops matching stays filed; only operator manual removals are respected. Whether a now-unmatching asset should be auto-removed is a product decision — **out of scope**, flagged for the operator. If ever wanted, it's a new branch in `fill_album`, not a trigger change.
6. **Pass-(b) decision churn.** Re-touching an asset (Immich bumped `updatedAt`) re-upserts its `asset_decisions` even if the verdict is unchanged — cheap, and keeps `decided_at` honest. No dedup planned.

---

## 12. Summary of decisions locked by this doc

| # | Decision |
|---|----------|
| 1 | Matching is event-driven; the indexer sweep is the only heartbeat; per-rule poll timers (`engine_scheduler.rs`) are deleted (L1). |
| 2 | Two reusable passes share one core: pass (a) full-index scan of one rule; pass (b) a touched asset-set × all of a user's active rules. Both reuse `evaluate_expr` + `fill_album` + `compute_album_plan` **unchanged** (L2). |
| 3 | A partial `matched` set in pass (b) is correct: `to_add` shrinks to newly-touched matches; `newly_removed` (= `prior_added − in_album`) stays full, so removal-respect (D3) holds. |
| 4 | Indexer hands each sweep's touched ids to pass (b) via an injected `OnSweepFn` seam (indexer stays storage-only, no `data_dir`/engine dep) (T40). |
| 5 | Rule create/activate/edit spawns pass (a) for that rule — immediate backfill, no tick wait (L3); `AppState.scheduler → matcher` service (T41). |
| 6 | One process-wide hourly safety task runs pass (a) over all active rules, `SummaryOnly` — the backstop for missed events (L4). |
| 7 | `rule_runs`: written by pass (a) (lifecycle + hourly) only; pass (b) writes none — kills the bursty-runs noise. Table kept; `/runs` endpoint removed iff unused. |
| 8 | `poll_interval_seconds` UI field + validator removed; column kept but unread (supersedes cycle-4 directive #6). |
| 9 | Deletion: low-cadence full-id reconcile in the indexer prunes `asset_index` + hand-cascades `asset_decisions` + `album_managed_assets` (scoped by owner); never touches the Immich album (T43). |
| 10 | Activity view = `/me/index/status` header + client-side per-asset grouping of the existing event stream; the only bus change is adding `asset_id` to `Indexed` (L5). D5 Chrome-MCP gate before T44 is done. |
| 11 | YOLO stays lazy + cached; `asset_index` schema + indexer sweep stay as cycle 5 left them (D1, `preprocessing-index.md`). |
