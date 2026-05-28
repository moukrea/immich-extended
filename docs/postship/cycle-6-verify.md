# POSTSHIP cycle 6 — live verification (event-driven matching + Activity view)

**Date:** 2026-05-28 (22:5x UTC ≈ 00:5x local CEST)
**Deploy:** `~/server/immich-extended` via `make up-immich-extended`; image `immich-extended:dev` (sha256:90b69850…), container `Up (healthy)`.
**Endpoint:** `https://immich-ext.${DOMAIN}/health` → HTTP 200 `{"status":"ok","version":"0.1.0","db":"ok"}`.

Cycle 6 retired rule-centric poll timers and made rule matching **event-driven** off the
background indexer sweep, and reshaped the Activity view into a library-status header plus a
per-asset grouped live log. This documents the live verification against the deployed binary
(TASKS.md §POSTSHIP-T45 items 1–4). Evidence is from the live SQLite DB
(`$IMMICH_EXT_DATA_PATH/immich-extended.sqlite`), the live Immich API (admin key), the running
container logs, and `docs/postship/cycle6-t45-activity-live-dark.png`.

All times are UTC; local screenshots show CEST (UTC+2).

---

## Item 1 — new rule activation fills its album IMMEDIATELY (no poll-tick wait)

A managed-album rule `iext-t45-verify` (id `bfad0cb9…`, target `aff2d39b-3e04-4efe-b93e-e31e43dc04b4`,
strategy `managed`, owner `447f13a3` = `smoke-local`) was created + activated at **22:24:15**.

On activation the T41 `on_rule_activated` hook fired and:
- **created + bound the managed album** — `target_album_id` is populated (`aff2d39b…`); the album
  did not exist before, so `resolve_target_album` created it in the owner's Immich and persisted
  the id back to the rule.
- ran a **full-index scan immediately** (not a watermark-limited poll): the rule's decision rows
  total **2007** — `added=6`, `skipped=2001` — i.e. every one of the user's 2007 indexed assets
  was evaluated at creation time (`created_at = last decided at 22:24:15`), with no wait for a
  poll interval.

End-to-end album fill confirmed against **live Immich** (admin key, `GET /api/albums/aff2d39b…`):

```
albumName = iext-t45-verify    assetCount = 6    ownerId = eb2d5112
```

`assetCount = 6` matches the 6 `added` decisions and 6 `album_managed_assets` rows (state
`added`). Per the cycle-5 T26 invariant, `added` is recorded only after the Immich PUT succeeds,
so the 6 added rows are 6 confirmed live PUTs. **Album filled on activation, no poll-tick wait.**

## Item 2 — change an asset → re-indexed + re-evaluated within ~1 sweep; membership reflects it

Controlled, fully-reversible live mutation on an already-indexed asset
`dc0cfe40…` = `IMG_20220409_161323.jpg`:

| | before mutation | after the 22:53:48 sweep |
|---|---|---|
| Immich `updatedAt` | 22:28:41 | 22:51:29 (bumped via `isFavorite` toggle at 22:51:29) |
| `asset_index.updated_at` | 22:28:41 | **22:51:29** (indexer read the new value) |
| `asset_index.indexed_at` | 22:49:47 | **22:53:48** (re-indexed) |
| owner `last_seen_updatedAt` watermark | 22:28:41 | **22:51:29** (advanced) |
| `asset_decisions.decided_at` (verify rule) | 22:49:47 | **22:53:48** (re-evaluated) |
| decision | `added` | `added` (still matches the date predicate → membership consistent, no churn) |

I bumped the asset's `updatedAt` (favorite toggle) at 22:51:29; the next indexer sweep (22:53:48,
within one ~120 s cadence) re-indexed it, advanced the watermark, and the matcher re-evaluated it
against all active rules — exactly the T40 indexer→matcher event wiring. The verdict recomputed
identically (the favorite flag does not affect the date predicate), so album membership stayed
consistent — no spurious add/remove. The `isFavorite` toggle was reverted to `false` afterward
(asset restored to its original state).

## Item 3 — Activity view: status header + coherent per-asset log; per-rule timers gone; hourly safety sweep present

Screenshot of the live deploy: `docs/postship/cycle6-t45-activity-live-dark.png`.

- **Status header** — `INDEXED 2 007 / 2 007 · LAST SWEEP 16s ago · idle`.
- **Per-asset grouped log** — cards keyed by asset, each `indexed · N people · GPS` folding into
  its verdicts, e.g. `VID_20250901_194044.mp4 indexed · 0 people · GPS → skipped "iext-t45-verify"
  · Date out of range` and `IMG_20220409_161323.jpg indexed · 0 people · GPS → matched
  "iext-t45-verify"`, interleaved with `Library sweep — indexed N asset(s)` summary rows. The log
  reads as the cycle-6 §L5 sentence, with filenames + row-height thumbnails (not raw UUIDs).
- **Per-rule poll timers retired (T42)** — `crates/server/src/engine_scheduler.rs` no longer
  exists (glob: no files); container logs contain **no** scheduler / per-rule / poll_interval
  lines, and there are no 5-second per-rule run bursts.
- **Indexer is the heartbeat** — steady 120 s sweeps in the logs
  (`indexer sweep complete` at 22:45:46 → 22:47:47 → 22:49:47 → 22:51:47 → 22:53:48).
- **Hourly safety sweep present (T42)** — startup logs:

```
indexer started        interval_secs=120
safety sweep started   interval_secs=3600
serving frontend from WEB_DIST_DIR=/app/web/dist
```

Light-theme parity of the Activity view was validated in the T44 D5 gate
(`docs/postship/cycle6-t44-activity-light.png`); this live shot is dark-theme.

## Item 4 — gates green

Cargo (`ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so.1 SQLX_OFFLINE=true`):
- `cargo fmt --all --check` ✓
- `cargo clippy --all-targets --workspace -- -D warnings` ✓
- `cargo test --workspace` ✓ — **482 passed / 0 failed**

Web (`cd web`):
- `npm run build` ✓
- `npm run lint` ✓
- `npm run typecheck` ✓
- `npm test -- --run` ✓ — **288 passed**

---

## Conclusion

Event-driven matching and the reworked Activity view are **live-verified** against the deployed
binary: rule activation fills the album immediately, asset changes are re-indexed and
re-evaluated within one indexer sweep, the Activity view shows a coherent status header + per-asset
grouped log with the per-rule poll timers gone and the hourly safety sweep running, and all gates
are green. M7 stays `[x]`; cycle 6 is functionally complete pending the file-trim + sentinel
close-out (TASKS.md §POSTSHIP-T45).
