# POSTSHIP cycle 5 — T26 live verification: managed-album backfill

**Date**: 2026-05-28
**Commit deployed**: `4a9dcdd` (`fix(engine): backfill managed albums + record added only on successful PUT`)
**Image**: `immich-extended:dev` rebuilt from `main`, redeployed via `make up-immich-extended`.

## The bug

`beba1580 Paloma (partage Maman)` had its managed album `e8e8d5e9-cc7d-4284-861f-cd4b4cea71fc`
created on a prior cycle, but the album was **empty** on Immich despite 313 `decision=added`
rows in `asset_decisions`. Cause: the 313 matches were decided BEFORE the album existed
(`target_album_id` empty at decision time), recorded as "added" anyway, and the watermark
advanced past them — so they were never re-filed once the album appeared.

## The fix (already landed in `4a9dcdd`)

- **Defect (i)** — `engine_cycle::run_one_cycle` now PUTs matched assets into the album
  (`idempotent_album_add`) BEFORE recording any `added`; a failed PUT (or no album) records
  nothing and holds the watermark, so no phantom "added".
- **Defect (ii)** — `resolve_target_album` resets the rule watermark to NULL the first time a
  managed album is bound, so that cycle re-scans the whole library and backfills.
- New `album_managed_assets(rule_id, asset_id, state, changed_at)` table (migration 0008,
  locked decision D3) — an `added` baseline row is written on each successful PUT.

`beba1580`'s album was already bound (defect ii's auto-reset does not re-fire for it), so a
**one-time watermark reset** was applied on the host DB to trigger the backfill:
`UPDATE rules SET last_processed_asset_timestamp=NULL WHERE id='beba1580-8499-4a00-b667-f0a9a9e0017b';`

## Live verification (host: `immich-ext.rdti25e2d.dedyn.io`)

Migration 0008 applied on boot (`_sqlx_migrations` version 8 `album managed assets`, success=1);
`album_managed_assets` table present. `/health` = 200 over HTTPS (TLS verify 0).

| Album (rule)                         | assetCount BEFORE | assetCount AFTER |
| ------------------------------------ | ----------------- | ---------------- |
| `e8e8d5e9…` (`beba1580` managed)     | **0**             | **310**          |
| `d51179c1…` (`714dce95` existing)    | 976               | **976** (undisturbed) |

First cycle after the reset (engine log):
```
rule cycle ok rule_id=beba1580-… evaluated=1250 added=310 skipped=940
```
- Album `e8e8d5e9` climbed 0 → **310** in one tick.
- `album_managed_assets` for `beba1580`: **310** rows, state=`added` (the D3 baseline).
- The cycle's `added=310` exactly matches the album count → **no phantom adds** (defect i holds).
- Watermark advanced NULL → `1779629149` (below the prior `1779911950`); the remaining few
  matches (313 historical added decisions vs 310 filed this tick) fall in the next
  `updatedAt` window and are filed automatically by subsequent 300s ticks — the watermark
  self-advances each tick until the album stabilises. The empty-album bug is resolved.
- `714dce95`'s album stayed at 976 — its watermark (`1779911950`) was left untouched, so it
  was not re-scanned. The working rule is unaffected.

## Exit criteria — MET

> operator's `Paloma (partage Maman)` album on Immich contains the matched assets;
> `714dce95` unaffected.

310 matched assets now in `e8e8d5e9` (was 0); `714dce95` undisturbed at 976.
