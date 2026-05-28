# POSTSHIP-T32 — Per-rule Activity rework (verify)

**Date:** 2026-05-28
**Scope:** Frontend (SPA bundle) + a small server addition (thumbnail proxy +
decisions `filename`/`decision` filter). The server bits need an image
rebuild+redeploy; the web bits ship in the same SPA bundle.

## What changed

Per cycle-5 LOCKED DECISION **D4** the per-rule page `/rules/:id/activity` is now
**decisions-only** (the global live log is the separate `/activity`, T33).

Server:
- **`GET /api/v1/me/assets/:id/thumbnail`** — cookie-auth, per-user proxy
  reusing `immich-client::download_thumbnail`, mirroring the existing
  `person_thumbnail` contract (`image/jpeg`, `Cache-Control: private,
  max-age=86400`). The Immich API key never reaches the browser.
- **Decisions payload carries `filename`** — `list_decisions_for_rule_filtered`
  now `LEFT JOIN`s `asset_index` (T28) on `(user_id, asset_id)`; `filename` is
  `null` for un-indexed / deleted assets (UI falls back to a short hash).
- **`?decision=added|skipped` filter** on the decisions endpoint backs the
  Matched/Skipped chips; an invalid verb returns `400 invalid_decision`.
- Dynamic query via `QueryBuilder` (binds only — injection-safe); no `.sqlx/`
  macro changed, so no offline-cache regen.

Web (`RuleActivity.tsx`, full rewrite):
- **Dropped the "Recent runs" panel** entirely (operator found it meaningless);
  removed `fetchRuleRuns`/`useLivePoll` usage from the page.
- **Header names the rule** — "Activity — &lt;rule name&gt;" (via `getRule`) with a
  "← Back to rule" link.
- **Decisions table**: fixed-height (`max-h-[28rem]`) container with the scroll
  INSIDE it and a pinned (`sticky`) header row — not whole-page scroll;
  lazy-loads the next page on scroll AND via a "Load more" button (both guarded
  by a monotonic request token so a filter switch can't be clobbered by a slow
  in-flight page); **All / Matched / Skipped** filter chips; each row shows the
  asset **filename + a row-height thumbnail** (proxied through `/me/assets/:id/
  thumbnail`) that **enlarges in a hover preview** (a `fixed`-positioned popup so
  it escapes the table's overflow clip) — the raw UUID is gone from the visible
  label (kept only as the row `title`).

## Gates

Rust (`ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so.1`):
- `cargo fmt --all --check` ✓
- `cargo clippy --all-targets --workspace -- -D warnings` ✓
- `cargo test --workspace` ✓ — all green (incl. new `common::decisions`
  filename/decision-filter unit test + `rule_decisions.rs`
  `decisions_carry_filename_and_filter_by_decision` integration test).

Web:
- `npm run typecheck` ✓
- `npm run lint` ✓ (0 warnings / 0 errors)
- `npm test -- --run` ✓ — **157 vitests / 16 files** (ruleActivity reworked
  5→6: header names rule + no Recent runs, filename+thumbnail proxy URL, chip
  filtering, lazy-load append, hover preview, load-error alert).
- `npm run build` ✓ — main **180.84 kB / 56.11 kB gzip**.

## D5 UI quality gate (mandatory)

No Chrome MCP tool in this harness, so the REAL `RuleActivity` page was rendered
inside the real `AppShell` at `/rules/:id/activity` via a throwaway Vite
`web/devpreview/` harness (`MemoryRouter` + `initialMe`; since removed) and
driven headless with Python Playwright (bundled chromium, `device_scale_factor=2`).
ALL `/api/v1/**` traffic — rule, decisions, and `<img>` thumbnails — was
fulfilled via Playwright route interception (PIL-generated gradient thumbnails),
so the real fetch/img code paths ran. Saved here:

- `cycle5-t32-activity-dark.png` — dark (Immich default), All filter, full table
- `cycle5-t32-activity-light.png` — light (class flipped)
- `cycle5-t32-activity-hover.png` — dark, first thumbnail hovered → enlarged preview
- `cycle5-t32-activity-skipped.png` — Skipped chip active → only skipped rows

**Critical comparison vs `docs/design/immich-style-mirror.md` §8.4 + the D4/T32 spec:**

- §8.4 sketched "Recent runs" + "Recent decisions"; the operator killed Recent
  runs (cycle-5 directive #6a), so this page is decisions-only with the rule
  named in the header — matches the revised intent. ✓
- Dark: near-black `#0a0a0a` body, `#212121` (`--immich-dark-gray`) card,
  `rounded-2xl`, surface-tone separation — matches §1.1 / §4.3. ✓
- Light: white card + hairline border, brand `#4250af` active chip — matches
  §4.7. ✓
- Filenames + row-height thumbnails replace the raw UUID; hover enlarges; pills
  read "matched"/"skipped"; reasons are humanized (`reasonLabel`). ✓
- Fixed-height table with internal scroll + pinned header (no whole-page
  scroll); "Load more" / "End of list — N shown" footer. ✓
- Sidebar Rules/Activity/Settings with "Rules" highlighted on this `/rules/:id/*`
  sub-page (T31 `matchPrefixes`); T30 account avatar top-right; no stray
  sign-out / identity line. ✓

Verdict: reads like an Immich management view, not a generic form, in both
themes; a non-technical operator sees filenames + thumbnails and a clear
matched/skipped filter instead of UUIDs.

## Deploy note

The web changes reach the operator on the next image rebuild (container still on
the T29 image). The two server additions (thumbnail proxy, decisions
`filename`/`decision`) ride the SAME rebuild — bundle with T33/T36 server work,
then `docker build` + redeploy once before the cycle-5 close-out (T37).
