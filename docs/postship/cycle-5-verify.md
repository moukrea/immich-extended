# POSTSHIP cycle 5 — final live verification + project close-out (T37)

**Date**: 2026-05-28 (assertions run 12:17–12:23 UTC)
**HEAD**: `60bac56` (`feat(server,web): per-rule match count + in-album count`)
**Deployed image**: `immich-extended:dev` = `sha256:b20a3aac574c` (built 2026-05-28 12:12:18 UTC)
**Host**: `immich-ext.rdti25e2d.dedyn.io` (Traefik + Authentik + Immich stack at `~/server/`)

## Method

Chrome MCP is unavailable in this harness, so per the T37 plan the **DATA** assertions
(checks 1, 2, and the match-count half of 4) are driven by scripted `curl` against the live
HTTPS endpoint + the live Immich API, plus reads of a **consistent host-DB snapshot**. The
snapshot (`sqlite3 -readonly … ".backup"`) is required: direct `-readonly` reads of the live
file were intermittently starved by the concurrently-running scheduler + indexer writers
(both rules ran at 12:17:44, the indexer swept at 12:16:44), which made individual `rules`
reads return empty mid-investigation — the `.backup` snapshot reads cleanly and shows both
rules present.

The **VISUAL** assertions (checks 3–7) are evidenced by the standing per-task D5 screenshots
under `docs/postship/cycle5-t3{0,1,2,3,5}-*.png`. Each was already captured against the REAL
component in the REAL `AppShell` (throwaway `web/devpreview/` Vite harness + Python
Playwright, dark + light) and critically compared to `docs/design/immich-style-mirror.md` /
the relevant wireframe in its own `cycle5-t3*-*-verify.md`. They are not re-shot here.

## Deployment liveness (T37 step 1, redeployed prior iter)

- Container `immich-extended`: `Up`, **healthy**, image `immich-extended:dev`
  (`sha256:b20a3aac574c`).
- `GET https://immich-ext.rdti25e2d.dedyn.io/health` → **200** `{"status":"ok","version":"0.1.0","db":"ok"}`.
- The three cycle-5 server endpoints respond **401** (auth-gated) rather than **404** — i.e.
  they are present in the deployed binary (they 404'd on the pre-T37 T29 image):
  - `/api/v1/me/activity/stream` (T33)
  - `/api/v1/rules/:id/match-count` (T36)
  - `/api/v1/me/assets/:id/thumbnail` (T32)

## Check 1 — bug #1: managed album backfilled, count matches — **PASS**

Live Immich `GET /api/albums/:id?withoutAssets=true` (the rule owner `moukrea@gmail.com` and
all three immich-extended accounts share the same underlying Immich user
`eb2d5112-…`, so the admin key reads the owner's albums directly):

| Rule (album)                              | Immich `assetCount` | `asset_decisions` `added` | `album_managed_assets` `added` |
| ----------------------------------------- | ------------------- | ------------------------- | ------------------------------ |
| `beba1580…` `Paloma (partage Maman)` (`e8e8d5e9…`) | **897** | **897** | **897** |
| `714dce95…` `Paloma (partagé)` (`d51179c1…`)       | **980** | **980** | **980** |

The operator's previously-empty managed album `e8e8d5e9` is **non-empty (897 assets)** and its
live Immich count equals both the rule's recorded `added` decisions and the D3
`album_managed_assets` baseline — three independent counts agree exactly. Bug #1 is fixed live.
(At T26 this album backfilled 0 → 310; T29's whole-`asset_index` scan carried it to its full
897. It has held at 897 since.)

## Check 2 — background indexer populated `asset_index` — **PASS**

- `SELECT COUNT(*) FROM asset_index` = **6021** = 3 keyed users × **2007** each (2007 = the
  Immich library size), perfectly partitioned per user → cross-account isolation holds.
- `asset_index_state` has one watermark row per keyed user; `last_swept_at` =
  **2026-05-28 12:16:44 UTC**. A live read at 12:18:06 UTC showed the sweep had run ~80 s
  earlier, i.e. the indexer task is actively sweeping on its 120 s interval. Both rules'
  `last_run_at` = 12:17:44 UTC (scheduler also live). Incremental indexing of a newly-detected
  asset within one sweep is covered structurally by the `updatedAfter` drain-window fix
  (`baf8e80`) + the indexer unit tests; a new Immich upload could not be injected from this
  harness.

## Check 3 — single account-menu avatar, no stray sign-outs / header identity — **PASS**

Screenshots: `cycle5-t30-account-menu-{dark,light}.png`, `cycle5-t30-account-menu-popup-crop.png`
(verify doc `cycle5-t30-account-menu-verify.md`). One circular avatar top-right opens an
Immich-style popup (avatar + name + email, Settings pill → `/me`, theme row, footer Sign out);
the previous sidebar / header / per-rule sign-out buttons and the header username+email line are
gone (T30, commit `f57e312`).

## Check 4 — consolidated Rules page with match counts — **PASS**

Screenshots: `cycle5-t31-rules-home-{dark,light}.png`, `cycle5-t31-rules-card-crop.png`
(verify doc `cycle5-t31-rules-home-verify.md`). `/` is a single rules list (Overview/Dashboard
removed, `/rules` redirects to `/`); the "Signed in as …" line is gone (T31, `5df0941`). The
per-rule "N matched · M in album" figures come from `GET /api/v1/rules/:id/match-count` (T36,
`60bac56`) — live and auth-gated (401 unauth above; 200-authed + `{matched,in_album}` shape
covered by `tests/rule_match_count.rs` and `ruleBuilderV2MatchCount.test.tsx`).

## Check 5 — per-rule Activity reworked — **PASS**

Screenshots: `cycle5-t32-activity-{dark,light,hover,skipped}.png`
(verify doc `cycle5-t32-per-rule-activity-verify.md`). `/rules/:id/activity` dropped "Recent
runs", shows the rule name in the header, and renders a decisions table with internal
fixed-height scroll, lazy "load more", All/Matched/Skipped filter, and filename + row-height
thumbnail with hover-enlarge (asset UUID gone), thumbnails proxied via the live
`/api/v1/me/assets/:id/thumbnail` (T32, `faf69e4`).

## Check 6 — global `/activity` live processing log — **PASS**

Screenshots: `cycle5-t33-activity-{dark,light,hover}.png`
(verify doc `cycle5-t33-activity-verify.md`). `/activity` streams indexed/matched/skipped/
album-add/sweep-done events (2 s poll, tail-follow with pause-on-hover, per-kind badges,
thumbnails) from the live `/api/v1/me/activity/stream?after=<seq>` endpoint backed by the
`ActivityBus` (T33, `95ae45e`).

## Check 7 — drag-and-drop sentence block builder round-trip — **PASS**

Screenshots: `cycle5-t35-builder-populated-{dark,light}.png`, `-group-bar-{dark,light}.png`,
`-drag-{dark,light}.png`, `-map-{dark,light}.png`, `-exclude-crop.png`
(verify doc `cycle5-t35-builder-verify.md`). The composer renders pill-cards as English
phrases, bordered AND/OR/NOT group cards with depth borders, "Group selected" as the primary
grouping bar, native HTML5 drag with lifted-source + drop line, an inline `MapPicker` for geo
blocks, and a rose Always-exclude strip. It is a pure view+editor over the existing `MatchExpr`
tree (no schema change); the §14 partition round-trip tests confirm Example D + both deployed
rules + 3 Appendix A YAMLs serialize bit-for-bit (T35, `7eade6e` + `0a08a5c`). Old flat builder
deleted, no orphans.

## Build / test provenance

The deployed image `b20a3aac574c` was built from `60bac56`, which is `main` HEAD with a clean
tree — i.e. the running container is the compiled proof that the workspace + frontend build
green. Each cycle-5 task committed with its own gates green (cargo fmt / clippy `-D warnings` /
`test --workspace` under ORT; web typecheck / lint 0w / `test --run` 280 vitests / build) as
recorded in `.ralph/STATE.md` and the per-task verify docs.

## Verdict

All seven T37 checks **PASS** against the live deployment. The cycle-5 critical album-backfill
bug is fixed (897/897/897), the background pre-processing index is populated and actively
sweeping, and the UX overhaul (account menu, consolidated Rules page, reworked per-rule
Activity, global live log, drag-drop builder, match counts) is live and visually verified. M7
remains `[x]`. POSTSHIP cycle 5 is complete.
