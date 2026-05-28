# POSTSHIP cycle 5 — T29 live verification: index-based matching + full album fill

**Date**: 2026-05-28
**Commit deployed**: `bc7a3be` (`feat(engine): match rules against the pre-processed index + full album fill`)
**Image**: `immich-extended:dev` rebuilt from `main`, redeployed via `make up-immich-extended` (container created 2026-05-28 07:49:00 UTC, `Up … (healthy)`).

## What T29 changed

The poll cycle no longer fetches an Immich `updatedAt > watermark` page per tick. Each
cycle now scans the rule owner's **entire** `asset_index` (T28), reuses `engine::evaluate_expr`
+ `decide_with_optional_yolo` unchanged, and reconciles the match set against the live album
via the pure `album_sync::compute_album_plan` (D3):

```
to_add = matched − in_album − removed_set − newly_removed
```

The per-rule watermark (`last_processed_asset_timestamp`) is retired from the matching path
(kept in schema, unused), so a match can no longer be stranded behind it — the empty-managed-album
bug (T26) is now structurally impossible. `added` rows are written to `album_managed_assets`
**only after the Immich PUT succeeds** (T26 invariant preserved); lazy-YOLO stays cached in
`asset_yolo_cache` (D1).

## Live verification (host: `immich-ext.<DOMAIN>`)

Container healthy on the T29 image; indexer running (`indexer started interval_secs=120`,
`indexer sweep complete users_swept=3`). Both rules now do a **full-library scan** every tick —
`evaluated=2007` (= the full per-user indexed library), not a watermark window:

```
07:54:02  rule cycle ok rule_id=714dce95… evaluated=2007 added=980 skipped=1027
07:57:19  rule cycle ok rule_id=beba1580… evaluated=2007 added=897 skipped=1110
07:59:02  rule cycle ok rule_id=714dce95… evaluated=2007 added=980 skipped=1027   ← cycle #2
```

> Note: the `added` log field counts *matched* assets (the desired album membership / decisions),
> NOT the number of new Immich PUTs. The real new-PUT count is `plan.to_add.len()`, which on a
> steady-state cycle is 0 (everything already in the album).

| Album (rule)                          | T26 result | T29 managed (`album_managed_assets`) | Live Immich `assetCount` |
| ------------------------------------- | ---------- | ------------------------------------ | ------------------------ |
| `e8e8d5e9…` (`beba1580` managed)      | 310        | **897**                              | **897**                  |
| `d51179c1…` (`714dce95` existing)     | 976        | **980**                              | **980**                  |

- **beba1580 "Paloma (partage Maman)"** — backfilled 310 → **897**. The original bug (album
  empty despite recorded `added` decisions) is resolved *and* the album now holds the complete
  match set the watermark path could never reach. `album_managed_assets` = live `assetCount` = 897.
- **714dce95 "Paloma (partagé)"** — completed 976 → **980** (4 matches whose `updatedAt` sat
  below the old watermark, never re-evaluated by the legacy path; the full scan finds them).
  Album owner unchanged (`eb2d5112…`); nothing removed. `album_managed_assets` = live `assetCount` = 980.

### Idempotency — PROVEN LIVE

`714dce95` cycled twice (07:54:02 and 07:59:02), both `added=980`. `album_managed_assets` for
`714dce95` kept `max(changed_at) = 07:54:02` across cycle #2 → **cycle #2 wrote zero new rows**
(`to_add = ∅`, `INSERT OR IGNORE` no-ops, no `changed_at` bump), and the live album held at 980.
`beba1580`'s live `assetCount` (897) equals its managed count (897), so its next cycle likewise
computes `to_add = ∅` via the identical code path.

### D3 manual-removal

`compute_album_plan` subtracts both prior `removed` and removals detected this pass
(`prior_added − in_album`); covered by unit tests (`crates/server/src/album_sync.rs`) and an
integration test seeding `asset_index`. No operator removals exist yet, so `newly_removed = ∅`
this pass — the path is exercised by tests, not yet by live data.

## Gates (re-run 2026-05-28, `ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so.1`)

- `cargo fmt --all --check` ✓
- `cargo clippy --all-targets --workspace -- -D warnings` ✓
- `cargo test --workspace` ✓ (all unit + integration binaries pass; tests reworked to seed
  `asset_index`, plus a D3 manual-removal test and index-scoped cross-account isolation)
- `cd web && npm run build` ✓ (backend-only task; bundle unchanged)

## Exit criteria — MET

> rewire matching + album-fill onto the index; beba1580 + 714dce95 produce correct albums under
> the new path; a second cycle is idempotent.

Both albums correct live (897 / 980, managed == live), full-library scan confirmed
(`evaluated=2007`), and idempotency demonstrated by two identical consecutive `714dce95` cycles
writing zero new rows. T26's stopgap watermark logic is superseded.
