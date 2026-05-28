# POSTSHIP-T31 — Merge Overview/Activity into the Rules page (verify)

**Date:** 2026-05-28
**Scope:** Frontend only (SPA bundle). Ships to the operator with the next image rebuild.

## What changed

- The global **Overview** (`Dashboard.tsx`, was `/`) and the **Rules** list
  (`RulesList.tsx`, was `/rules`) were near-duplicates. They are now ONE
  consolidated Rules-centric home:
  - `/` renders the consolidated Rules page (`RulesList.tsx`).
  - `/rules` → `<Navigate href="/" />` (kept so old links/bookmarks resolve).
  - `Dashboard.tsx` + `dashboard.test.tsx` deleted (its last-run summary + live
    freshness folded into the Rules page).
- Per rule the card now shows: **status dot + name + status badge**, target
  album strategy (managed/existing), a **match-count placeholder slot**
  (`— matched`, filled by T36), the **last-run summary** (relative time, +added,
  skipped, or error / "No runs yet"), **lifecycle controls** (Edit, Pause/Resume,
  Archive, Delete with confirm dialogs), and an **Activity →** link to
  `/rules/:id/activity`. Archived rules render dimmed. "New rule" affordance kept.
- The redundant **"Signed in as <email> (<name>)" line is removed** — identity
  now lives only in the T30 account menu. (The `/me` Settings page legitimately
  keeps its Account identity card; out of T31 scope.)
- Per D4 the sidebar nav is now **Rules (`/`)**, **Activity (`/activity`)**,
  **Settings (`/me`)**. New `Activity.tsx` placeholder page exists at `/activity`
  (honest "live log is on its way" empty state) so the nav destination resolves;
  T33 fills its body. `SidebarNav` gained `matchPrefixes` so "Rules" (href `/`)
  stays highlighted on its `/rules/:id` sub-pages.

## Gates (web)

- `npm run typecheck` ✓
- `npm run lint` ✓ (0w / 0e)
- `npm test -- --run` ✓ — **156 vitests across 16 files** (was 152 / 17 at T30;
  removed `dashboard.test.tsx`, expanded `rulesList.test.tsx` 4→11, +1 SidebarNav
  `matchPrefixes` test).
- `npm run build` ✓ — main **181.40 kB / 55.66 kB gzip** (was 183.07 / 55.78 at
  T30; slightly smaller after dropping Dashboard).

## D5 UI quality gate (mandatory)

No Chrome MCP tool in this harness, so the REAL consolidated page was rendered
inside the real `AppShell` at path `/` via a throwaway Vite `web/devpreview/`
harness (`MemoryRouter` + stubbed API; since removed) and driven headless with
Python Playwright (bundled chromium), dark + light. Saved here:

- `cycle5-t31-rules-home-dark.png` — dark (Immich default): sidebar + cards + avatar
- `cycle5-t31-rules-home-light.png` — light (class flipped)
- `cycle5-t31-rules-card-crop.png` — tight crop of the active rule card (dark)

**Critical comparison vs `docs/design/immich-style-mirror.md` §8.1 wireframe:**

- Dark body near-black `#0a0a0a` with `#212121` (`--immich-dark-gray`) rule cards,
  `rounded-2xl`, surface-tone separation (no hairlines) — matches §1.1 / §4.3. ✓
- Layout mirrors the §8.1 wireframe: "Rules" + subtitle header, right-aligned
  "New rule" primary button, stacked rule cards with status dot, name, status pill,
  and a "Last run … · +N added · N skipped" sub-line. ✓
- Primary "New rule" button = brand `#4250af` + white text (light) / `#accbfa` +
  near-black text (dark), `shadow-md shadow-primary/20` — matches §4.7. ✓
- Sidebar item active state = `bg-primary/10 text-primary`; "Rules" highlighted
  (path `/`). T30 avatar (initials "EM") sits top-right. ✓
- No "Signed in as" line anywhere on the page. ✓

Verdict: reads like an Immich management list, not a generic form, in both themes.
Web-only change — reaches the operator on the next image rebuild (container still
on the T29 image); T31–T36 will be bundled before redeploy.
