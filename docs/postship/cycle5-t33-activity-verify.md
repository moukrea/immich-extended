# POSTSHIP-T33 — Global live activity log (verify)

**Date:** 2026-05-28
**Scope:** Frontend (SPA bundle) + server (`ActivityBus` ring buffer +
`GET /api/v1/me/activity/stream` + indexer/engine emit points). The server bits
need an image rebuild+redeploy; the web bits ship in the same SPA bundle.

Code landed in `95ae45e` (committed + pushed). This doc records the mandatory
**D5 UI quality gate** for that code.

## What changed (recap)

Per cycle-5 LOCKED DECISION **D4**, `/activity` is the **global live processing
log** — NOT a duplicate of the rules list (directive #5).

Server:
- **`crates/server/src/activity.rs` — `ActivityBus`**: a bounded
  `Mutex<VecDeque>` + `AtomicU64` seq (cap **500** global, oldest evicted; seq
  assigned under the buffer lock so the deque stays ordered under concurrent
  publishers; poisoned-lock recovery via `into_inner()` — no `unwrap`).
  `ActivityEvent { seq, at, #[serde(skip)] user_id, #[serde(flatten)] kind }`;
  `ActivityKind` is `#[serde(tag="kind", rename_all="snake_case")]` =
  `indexed` / `matched` / `skipped` / `album_add` / `sweep_done`. `user_id` is
  `#[serde(skip)]` so it never reaches the wire.
- **`GET /api/v1/me/activity/stream?after=<seq>`** (cookie-auth `/me/*`) →
  `{events, last_seq}`, filtered per-user + `seq > after`.
- **Emit points**: `indexer.rs::sweep_one_user_inner` (Indexed per asset +
  SweepDone); `engine_cycle.rs::cycle_body` (Matched/Skipped per decision +
  AlbumAdd when `filled > 0`). Bus threaded via `Indexer::new(.., activity)`,
  `Scheduler::new(.., activity)`, `production_tick_fn(.., activity)`,
  `AppState.activity`. Public `sweep_one_user` / `run_one_cycle` delegate with
  `None` so the ~38 existing indexer/engine test call-sites are untouched.

Web (`Activity.tsx`):
- `useLivePoll` every 2 s; dedup + append by `seq`, cap 200 client-side;
  tail-follow auto-scroll with **pause-on-hover** (shows "Paused — move away to
  resume", `data-testid=activity-paused`).
- Per-`kind` rows: time + colored kind badge; `/me/assets/:id/thumbnail` thumbs
  for matched/skipped; humanized skip reasons (`reasonLabel`); honest idle empty
  state (`activity-empty`).
- `api.ts`: `ActivityEvent` discriminated union + `fetchActivityStream(after)`.

## Gates

Rust (`ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so.1`):
- `cargo fmt --all --check` ✓
- `cargo clippy --all-targets --workspace -- -D warnings` ✓
- `cargo test --workspace` ✓ — incl. `activity.rs` 4 unit tests +
  `me_activity.rs` 3 integration tests (happy-path + cursor, per-account
  isolation, 401-unauth).

Web:
- `npm run typecheck` ✓
- `npm run lint` ✓ (0 warnings / 0 errors)
- `npm test -- --run` ✓ — **161 vitests / 17 files** (+4 `activity.test.tsx`:
  append-across-polls + cursor, dedup-by-seq, idle empty state, pause-on-hover).
- `npm run build` ✓ — main **187.67 kB / 57.71 kB gzip**.

## D5 UI quality gate (mandatory)

No Chrome MCP tool in this harness, so the REAL `Activity` page was rendered
inside the REAL `AppShell` at `/activity` via a throwaway Vite `web/devpreview/`
harness (`MemoryRouter` seeded to `/activity` via `createMemoryHistory` +
stubbed `initialMe`; since removed) and driven headless with Python Playwright
(bundled chromium, `device_scale_factor=2`). **All** `/api/v1/**` traffic was
fulfilled via a single `page.route` dispatcher so the real fetch/img code paths
ran:

- `/api/v1/me/activity/stream?after=<seq>` returned a scripted 16-event log
  covering **every kind** (indexed / matched / skipped / album_add /
  sweep_done), honoring the `after` cursor (subsequent polls return the tail
  only), so the dedup-by-seq + tail-follow paths execute.
- `/api/v1/me/assets/:id/thumbnail` returned PIL-generated gradient JPEGs keyed
  by asset id, so each matched/skipped row shows a distinct real `<img>`.

Saved here:
- `cycle5-t33-activity-dark.png` — dark (Immich default), full page.
- `cycle5-t33-activity-light.png` — light (class flipped).
- `cycle5-t33-activity-hover.png` — dark, log hovered → "Paused — move away to
  resume" visible (card crop).

**Critical comparison vs `docs/design/immich-style-mirror.md`:**

- Dark: near-black `#0a0a0a` body, `#212121` (`--immich-dark-gray`) card,
  `rounded-2xl`, separation by surface tone not hairlines — matches §1.1 / §4.3. ✓
- Light: white card + hairline border, brand `#4250af` accents on the active
  nav + album badge — matches §4.7 / §4.3. ✓
- Live "ping" dot uses brand `immich-primary` / `dark:immich-dark-primary`
  (light-blue `#accbfa` in dark) — reads as a live indicator, not decoration. ✓
- Kind badges are pill-shaped `ring-inset` chips (Immich chip idiom): matched =
  emerald/success-green, album = brand-blue tint, indexed/skipped/sweep =
  neutral gray — a clear, calm hierarchy. ✓
- Rows show the **filename** (IMG_0101.jpg, IMG_0202.mp4) + a row-height
  thumbnail + humanized reasons ("Missing required person", "Date out of
  range") — NOT raw UUIDs. indexed rows read "1 person · GPS"; album rows
  "filed N assets into its album"; sweep "Library sweep — indexed N assets". ✓
- Fixed-height (`max-h-[32rem]`) internal scroll with tail-follow: the card
  shows the newest tail (seq 6→16) with the earliest rows scrolled above — the
  page itself doesn't grow. ✓
- Pause-on-hover proven live: hovering the log surfaces "Paused — move away to
  resume" in the header (the hover crop). ✓
- Shell: sidebar Rules / **Activity** (highlighted) / Settings (T31
  `matchPrefixes`); T30 account avatar ("OP") top-right; no stray sign-out /
  identity line; theme toggle lives only in the account menu. ✓

Verdict: reads like an Immich **processing log** — a live, scannable stream of
what the indexer and rule cycles are doing — not a generic activity feed and not
a duplicate of the rules list (directive #5 satisfied). Holds up in both themes.

## Deploy note

The web change reaches the operator on the next image rebuild (container still
on the T29 image). The server additions (`ActivityBus` + `/me/activity/stream` +
emit points) ride the SAME rebuild — bundle with the committed T32 server work
(thumbnail proxy + decisions `filename`/`decision`) and T36, then `docker build`
+ redeploy once before the cycle-5 close-out (T37). Until then, `/activity` on
the live deploy 404s the stream endpoint and the SPA shows "Could not reach the
activity stream. Retrying…" — expected; this D5 used the devpreview harness, not
the live deploy.
