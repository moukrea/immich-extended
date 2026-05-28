# POSTSHIP-T35 — Drag-and-drop block-sentence builder (D5 verify)

**Date:** 2026-05-28
**Scope:** Frontend only (SPA bundle). The visual rule builder was rewritten
from the "glorified form" (cycle-5 directive #7) into the D6 interaction model:
pill-cards that read as English, bordered AND/OR/NOT group containers,
"Group selected" as the primary grouping mechanism, drag-to-reorder, and a
top-level "Always exclude" strip. No `MatchExpr` schema change — the builder is
a pure view+editor over the existing IR (`matchTree.ts`).

Code landed across `4af459c` (PillCard) · `4004594` (NodeView/GroupCard) ·
`0c806ab` (SelectionBar/ExcludeStrip) · `7eade6e` (BlockTreeEditor composer +
deletion of the old flat builder) · `42e1566` (treeOps) · `e835752` (phrases) ·
`0a08a5c` (partition round-trip tests). This doc records the mandatory **D5 UI
quality gate** (LOCKED DECISION D5) for that work, plus a one-line dedup found
during the gate.

## Gates (HEAD, web-only change)

- `npm run typecheck` ✓
- `npm run lint` ✓ (0 warnings / 0 errors)
- `npm test -- --run` ✓ — **275 vitests / 24 files** (treeOps 38, phrases 28,
  partition round-trip 12, pillCard/nodeView/selectionBar/excludeStrip +
  ruleBuilderV2 component suites).
- `npm run build` ✓ — main **203.00 kB / 62.37 kB gzip** (down a hair from
  203.10 after removing the duplicate label); lazy `MapPicker` chunk unchanged.

## D5 UI quality gate (mandatory)

No Chrome MCP tool in this harness, so the **real** `RuleBuilderV2` was rendered
inside the **real** `AppShell` via a throwaway Vite harness (`web/devpreview.html`
+ `web/devpreview/main.tsx`, `MemoryRouter` seeded at `/rules/:id`; since removed)
and driven headless with Python Playwright (bundled chromium,
`device_scale_factor=2`). All `/api/v1/**` traffic was served from an in-page
`window.fetch` monkeypatch so the real fetch → `request()` → `yamlToFormStateV2`
→ partition → render paths all ran; the rule `yaml_source` was generated through
the **real** `formStateToYamlV2` so the load round-trip reproduces the seeded
tree. Two rules were seeded:

- **`example-d`** — the operator's directive sentence (§9.1): *Include when
  ( Paloma AND count=1 ) OR ( Paloma AND Emeric AND count≥2 ), always exclude
  Manon* → `and([ or([ and([Paloma, =1]), and([Paloma, Emeric, ≥2]) ]),
  person{must_exclude, Manon} ])`.
- **`geo`** — `and([ Paloma, date_range, location, media_type ])` to exercise the
  location pill + inline `MapPicker` and a flat drag-reorder list.

People (Paloma / Emeric / Manon / Maman) were served so the phrases resolve real
names, not short ids.

Saved here (all `docs/postship/`):

- `cycle5-t35-builder-populated-dark.png` / `-populated-light.png` — §9.1 in both
  themes: the OR root group with two AND subgroups, pill-cards, depth borders,
  connector chips, the rose exclude strip with the Manon chip.
- `cycle5-t35-builder-group-bar-dark.png` / `-group-bar-light.png` — §9.3: two
  sibling pills ticked (primary ring), the sticky "2 selected · Group as ·
  AND | OR · Clear" action bar.
- `cycle5-t35-builder-drag-dark.png` / `-drag-light.png` — §9.4: a `dragstart`
  on the Paloma pill (dimmed `opacity-50`) + `dragover` on a lower drop gap →
  the 2px primary drop line between the location and media pills.
- `cycle5-t35-builder-map-dark.png` / `-map-light.png` — §9.5: the location pill
  expanded ("Hide map ▴") with the live `MapPicker` (real MapLibre + OSM tiles
  on Paris) and the radius slider.
- `cycle5-t35-builder-exclude-crop.png` — §5.5: the rose "Always exclude" lane
  cropped (🚫 Manon ✕ chip + dashed "+ add a person").

**Critical comparison vs §9 wireframes + `docs/design/immich-style-mirror.md`:**

- **Reads as a sentence, not a form.** Each leaf is a single-row phrase:
  "👤 Paloma ▾ is present", "🔢 people count [= equals ▾] [1]", "📅 taken from
  15/07/2024 to 22/07/2024", "📍 within 60 km of (48.8566, 2.3522) [Map ▾]",
  "🎞 is a photo". The variable parts are inline controls; the connective words
  are plain text. A non-technical operator can read the rule aloud. ✓ (directive
  #7 / D6).
- **Filled cards, not dashed drafts.** Pills are `rounded-xl` filled surfaces
  (`bg-white` / `dark:bg-immich-dark-gray`); group cards are `rounded-2xl` with a
  depth-colored `border-l-4` (primary → info → success → warning). The old
  `border-2 border-dashed` "draft" look is gone. ✓ (§10 / mirror §1.1).
- **Dark surfaces.** Near-black `#0a0a0a` body, `#212121` cards; separation by
  surface tone, not hairlines — matches the mirror. Light theme: white cards +
  hairline borders + brand `#4250af` Save button / AND toggle. ✓
- **AND/OR/NOT semantics are visible.** Segmented AND (`immich-primary` blue) /
  OR (`amber-500`) toggle per group, a NOT checkbox, connector chips between
  siblings, the OR root wrapping two AND subgroups exactly as the directive
  sentence parses. ✓
- **"Group selected" is the primary grouping path** (D6): ticking 2 siblings
  surfaces the dark sticky action bar with AND (blue) / OR (amber); the selected
  pills get the primary ring. Deterministic + keyboard-operable. ✓
- **Drag reorders** with the lifted-source + drop-line affordance (§9.4); the
  ▲▼ move gutter is the a11y fallback (§13) so drag is never the only path. ✓
- **"Always exclude" strip** is a distinct rose blacklist lane below the
  composer; the `must_exclude` Manon child is lifted out of the positive body
  into a removable chip (the §3 partition), reading "never matched, even if
  everything else fits." ✓
- **Inline location map** (§5.6): "Map ▾" expands the real `MapPicker` beneath
  the pill, center + radius bound to the leaf. ✓
- **Single section label.** Found + fixed during this gate: the composer was
  printing its own "Include media when" `<p>` on top of the host `Card`'s
  `<h2>` (duplicate). Removed the composer's copy — the host heading +
  description now sit once, directly above the OR group, matching §9.1.

Verdict: the builder reads like an **Immich management surface composing an
English sentence**, in both themes — not another flat form. Directive #7 and D6
(pill-cards + bordered AND/OR/NOT groups + Group-selected primary + drag-reorder
+ Always-exclude strip + inline map) are satisfied. Iterated within T35 (the
dedup) per D5 step 3.

## Deploy note

Web-only change: it reaches the operator on the next image rebuild (container
still on the T29 image). Bundle with the committed T32/T33 server work
(thumbnail proxy + decisions filter; `ActivityBus` + `/me/activity/stream`) and
T36, then `docker build` + redeploy **once** before the cycle-5 close-out (T37).
This D5 used the throwaway devpreview harness, not the live deploy. The dev
server was stopped by its `ss -ltnp | grep :5174` listener PID (not `pkill -f` —
the T31 self-match gotcha); port confirmed free afterward.
