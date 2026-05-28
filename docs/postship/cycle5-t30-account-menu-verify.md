# POSTSHIP-T30 — Immich-style account menu (verify)

**Date:** 2026-05-28
**Task:** Replace the three redundant sign-out surfaces (header username+email+signout,
sidebar sign-out, per-rule-page sign-out) with ONE circular avatar button top-right that
opens an Immich-style account popup. Move the theme toggle into the popup.

## What changed

- **New** `web/src/components/AccountMenu.tsx` — circular avatar button (initials, top-right)
  that toggles a popup. Popup mirrors Immich's account panel:
  - centered large avatar (initials), name (semibold), email (muted)
  - a bordered **Settings** pill → `/me`
  - a **Theme** row hosting the existing `<ThemeToggle>` (moved here from the header)
  - a divided footer with a full-width **Sign out** (danger-tinted hover)
  - closes on click-outside, Esc, Settings-follow; `aria-haspopup="menu"` + `aria-expanded`.
- `TopBar.tsx` — dropped the inline `<ThemeToggle>`, the username+email identity block, and
  the `topbar-signout` button; renders `<AccountMenu>` only.
- `AppShell.tsx` — removed the desktop sidebar sign-out and the mobile-drawer sign-out
  (the `signOut` callback is still wired through `TopBar` → `AccountMenu`).
- `RuleBuilderV2.tsx` — removed the per-rule-page "Sign out" button + `onLogout` + the now
  unused `postLogout` import (the page header keeps "← Rules" / Activity / Decisions).

## Gates (web)

- `npm run typecheck` ✓
- `npm run lint` ✓ (0w / 0e)
- `npm test -- --run` ✓ — **152 vitests / 17 files** (was 132 / 15; +8 `accountMenu.test.tsx`
  covering open/close, identity, Settings href, sign-out callback, theme flip, Esc,
  click-outside, Settings-follow; the `appShell.test.tsx` sign-out/identity tests were
  reworked to drive the account menu, +1 regression test asserting NO stray
  `topbar-signout` / `sidebar-signout`).
- `npm run build` ✓ — main bundle 183.07 kB / 55.78 kB gzip.

## D5 UI quality gate (mandatory)

Rendered the real `AccountMenu` component via a throwaway Vite harness
(`web/devpreview/`, since removed) served by the dev server, driven headless with
Playwright (bundled chromium) — clicked the avatar, screenshotted the open popup in both
themes. Saved into this folder:

- `cycle5-t30-account-menu-dark.png` — dark mode (Immich's default), full top bar + popup
- `cycle5-t30-account-menu-light.png` — light mode (theme toggled in-popup)
- `cycle5-t30-account-menu-popup-crop.png` — tight crop of the dark popup card

**Critical comparison vs `docs/design/immich-style-mirror.md`:**

- Dark surface is near-black `#0a0a0a` body with a `#212121` (`--immich-dark-gray`) card —
  matches §1.1 "near-black, cards #212121". ✓
- Card radius `rounded-3xl` (24px) — matches §4.4 modal/panel radius and Immich's account
  panel. ✓ Reads like Immich's account popup, not a generic dropdown.
- Avatar initials use `--immich-dark-primary` `#accbfa` (dark) / `--immich-primary` `#4250af`
  (light) — matches §1.1. ✓
- "Settings" rendered as a bordered pill (Immich's "Account Settings" affordance), not a bare
  menu row. ✓
- Theme toggle correctly flips the whole document and swaps sun↔moon. ✓
- Full-width Sign out in a top-bordered footer section, mirroring Immich's separated sign-out.
  ✓

Verdict: matches the Immich style mirror in both themes; ships via the next image rebuild.
The container is still on the T29 image — T30 (web-only) reaches the operator with the next
rebuild bundling T31–T36; no redeploy was required to close T30 (D5 screenshot is of the
real component code).
