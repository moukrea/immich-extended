# POSTSHIP-T44 — Asset-centric Activity view (D5 visual gate)

**Date:** 2026-05-29
**Scope:** Frontend `web/src/pages/Activity.tsx` + `web/src/lib/activityGrouping.ts`
(grouped log + status header) consuming the cycle-6 server slice
(`GET /api/v1/me/index/status` + `asset_id` on `Indexed` events).

Code landed in `d676fc4` (server slice 1) + `5515110` (frontend slice 2), both on
`origin/main`. This doc records the mandatory **D5 UI quality gate** — vitest-green
is explicitly NOT sufficient (§8.4 / cycle-6 D5).

## What changed (recap)

Per cycle-6 LOCKED DECISION **L5**, `/activity` = **library-status header** +
**per-asset grouped live narrative** (NOT a flat per-event feed, NOT a duplicate
of the rules list).

- **`activityGrouping.ts`** — pure `groupActivity(events) -> ActivityRow[]`:
  folds each asset's `indexed` + `matched` + `skipped` events (correlated by the
  new `asset_id`) into one `AssetGroup` card, keyed at the asset's first-seen
  `seq` so the narrative reads top-to-bottom in arrival order; verdicts collapse
  to latest-per-rule (Map insertion order). `album_add` / `sweep_done` stay as
  standalone interleaved `SummaryLine`s.
- **`Activity.tsx`** — `<StatusHeader>` (from `/me/index/status`: `Indexed N / M`,
  `Last sweep <ago>`, `idle|indexing` pill) above the grouped log. Each
  `<AssetCard>` shows time + row-height thumbnail (hover-enlarges) + filename +
  `indexed · N people · GPS` + `<VerdictChip>`s (emerald matched / slate skipped
  + humanized `reasonLabel`). 2 s stream poll + 10 s status poll, tail-follow,
  pause-on-hover, `MAX_EVENTS=200`, dedup-by-seq.

## Gates (automated)

Web (`cd web`):
- `npm run typecheck` ✓
- `npm run lint` ✓ (0 warnings / 0 errors)
- `npm test -- --run` ✓ — **288 vitests** (+7 `activityGrouping.test.ts` reducer,
  +1 net `activity.test.tsx`).
- `npm run build` ✓ — main 208.24 kB / 63.90 kB gzip.

Server slice (`d676fc4`): `cargo test --workspace` **482 passed** (incl.
`tests/index_status.rs` counts + null `library_total` + per-account scope, and
`activity.rs` `asset_id` serialization).

## D5 UI quality gate (mandatory)

No Chrome MCP tool is exposed in this harness (same as cycle-5 T32/T33), so the
**REAL built SPA bundle** (`web/dist` from `npm run build`, served by
`vite preview --host 127.0.0.1 --port 4173`) was driven headless with Python
Playwright (bundled Chromium, `device_scale_factor=2`, 1440×1000). This renders
the production bundle — what actually ships — not a devpreview stub. **All**
`/api/v1/**` traffic was fulfilled via a single `page.route` dispatcher so the
genuine fetch / grouping / `<img>` code paths execute:

- `/api/v1/setup/state` + `/api/v1/auth/me` → an authed session that lands on
  `/activity` (no redirect, per `decideBootstrapNavigation`).
- `/api/v1/me/index/status` → `{indexed: 1240, library_total: 1250,
  last_swept_at: <8s ago>, sweeping: true}` → header reads
  `Indexed 1 240 / 1 250 · Last sweep 9s ago · indexing`.
- `/api/v1/me/activity/stream?after=<seq>` → a scripted 13-event sweep over **4
  assets** and **2 rules** ("Paloma (partage Maman)", "Trip 2024") covering every
  kind (indexed / matched / skipped / album_add / sweep_done), honoring the
  `after` cursor (the tail poll returns empty), so dedup + tail-follow run.
- `/api/v1/me/assets/:id/thumbnail` → distinct PIL gradient JPEGs keyed by
  asset id, so each card shows a real, different `<img>`.

Saved here:
- `cycle6-t44-activity-dark.png` — dark (Immich default), full page.
- `cycle6-t44-activity-light.png` — light (class flipped), full page.
- `cycle6-t44-activity-hover.png` — dark, first thumbnail hovered → 192px enlarge
  preview + "Paused — move away to resume" in the log header.

**Critical comparison vs L5 narrative (§8.2):**

The L5 target line is
`IMG · indexed (3 people · GPS) → matched "Paloma" · skipped "Trip" (date out of range)`.
The top card renders **exactly** that:
`IMG_2942.jpg  ·  indexed · 3 people · GPS` then chips
`matched "Paloma (partage Maman)"` + `skipped "Trip 2024" · Date out of range`.
The grouping is correct end-to-end: IMG_2943 folds two skips ("Missing required
person", "Location missing GPS") onto one card; IMG_2944.mp4 folds two matches;
the `album_add` ("Paloma (partage Maman) filed 2 assets into its album") and
`sweep_done` ("Library sweep — indexed 4 assets") interleave as summary lines.
The numbers are internally coherent (2 Paloma matches → "filed 2"; 4 indexed →
"indexed 4"). Reads as a per-asset story, not a flat firehose. ✓

**Critical comparison vs `docs/design/immich-style-mirror.md`:**

- Dark: near-black `#0a0a0a` body with `#212121` (`--immich-dark-gray`) surface
  cards (status header + log), `rounded-2xl`, separation by surface tone — §1.1 /
  §4.3. ✓
- Light: white cards + hairline border, brand `#4250af` on the active nav item —
  §4.3 / §4.7. ✓
- Eyebrow labels (`INDEXED`, `LAST SWEEP`) uppercase + `tracking-wider` muted —
  §2.3. ✓
- Verdict + status chips are pill-shaped `ring-inset` (Immich chip idiom):
  matched = success-emerald, skipped = neutral slate, indexing = brand-primary
  tint — a calm, scannable hierarchy. §1.2. ✓
- Live "ping" dot uses brand `immich-primary` / `dark:immich-dark-primary` — reads
  as a liveness indicator, not decoration. ✓
- Rows show the **filename** + a row-height thumbnail + humanized reasons — NOT
  raw asset UUIDs. ✓
- Shell intact: sidebar Rules / **Activity** (highlighted) / Settings (T31);
  account avatar "OP" top-right (T30); no stray sign-out / identity line; theme
  toggle lives only in the account menu (cycle-5). ✓
- Hover proven live: the first thumbnail enlarges to a 192px fixed preview and the
  log header flips to "Paused — move away to resume" (the hover shot). ✓

The idle empty state (`activity-empty`, "Nothing processing right now") is a
trivial centered message and is covered by `activity.test.tsx`; not separately
screenshotted because the populated narrative is the artifact under review.

**Verdict:** the view reads like an Immich **processing log** — a live,
per-asset-grouped account of what the indexer and rule cycles are doing — and
holds up in both themes. No JSX iteration was needed; the shipped `5515110` JSX
matches L5 and the style mirror. **D5 PASS.**

## Deploy note

This D5 used the local production-bundle preview, not the live deploy. The
cycle-6 web bundle + server changes (T39–T44) reach
`https://immich-ext.<DOMAIN>` only on the **T45** rebuild+redeploy (on the new
model), where the event-driven path is live-verified end-to-end. Until then the
deployed `/activity` runs the cycle-5 bundle.
