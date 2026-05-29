# POSTSHIP-T48 — Inline condition pills for count/faces/date/media (D5 gate)

**Date:** 2026-05-29
**Scope:** Pure frontend. Extends `web/src/components/blocks/InlineSentenceBuilder.tsx`
(the cycle-7 inline sentence builder) so the remaining leaf types are editable
inline, and turns the "+ condition" affordance into a leaf-type menu. No engine
/ schema / API changes (the builder still emits the same `yaml_source`). Design
contract: `docs/design/inline-sentence-builder.md` §4.

This records the mandatory **D5 UI quality gate** (cycle-7 LOCKED DECISION L5):
vitest-green is explicitly NOT sufficient — the UI was screenshotted from the
real production component and critically compared to the design + the Immich
style mirror before commit.

## What shipped in T48

- **`ConditionPill` now opens an inline editor for every editable leaf**, not
  just `person`:
  - **people_count** — operator `<select>` (= / ≠ / < / ≤ / > / ≥ with words) +
    a non-negative integer input.
  - **face_recognition** — two checkboxes ("all faces must be recognized" ↔
    `!allow_unrecognized`; "also reject extra humans (YOLO)" ↔
    `yolo_count_check`).
  - **date_range** — two `<input type=date>` (From / To) emitting
    `T00:00:00Z` / `T23:59:59Z` ISO bounds, omitting an empty bound.
  - **media_type** — a three-way `<select>` (photo / video / photo or video).
  - **person** — unchanged from T47 (mode dropdown + reused `PersonPicker`).
  - **location** — intentionally **read-only** ("taken in an area"); its map
    picker + numbered Area linking is T50. The pill button is `disabled`.
  Every editable pill shows the `▾` affordance; the at-rest phrase comes from
  `leafSentence` (the single wording source, untouched).
- **"+ condition" is now a 6-leaf-type menu.** Reuses `AddBlockDropdown`
  (`groupKinds={[]}`, new optional `triggerClass` to render it as the dashed
  in-line pill) + `defaults.ts` `defaultLeaf(kind)`; `addPrimaryPerson`
  generalized to `addPrimaryPill(kind)`. Reusing `AddBlockDropdown` here also
  keeps it from being orphaned when T52 deletes the old composer.
- The editor/menu helpers (`OP_SELECT_LABEL`, `isoToInput`/`inputToIso`,
  `mediaSelectValue`/`mediaTypesFromValue`, `clampNonNegInt`) now live in
  `InlineSentenceBuilder.tsx` — their previous home `PillCard.tsx` is deleted in
  T52.

## Automated gates (web)

`cd web`:
- `npm run typecheck` ✓
- `npm run lint` ✓ (0 warnings / 0 errors)
- `npm test -- --run` ✓ — **318 vitests across 28 files** (was 313; +5 in
  `blocks/__tests__/inlineSentenceBuilder.test.tsx`: the 6-type menu, a
  people_count op+value edit + YAML round-trip, a face_recognition toggle, a
  date_range ISO-bound emit, a media_type photo→both switch, and a location
  read-only assertion; the T47 person/regression tests were rewritten onto the
  menu flow, as was `ruleBuilderV2.test.tsx`'s `addPerson` helper).
- `npm run build` ✓ — main `192.54 kB / 59.31 kB gzip` (was 185 kB at T47;
  +~7 kB for the five inline editors + the leaf-type menu reuse).

No Rust files changed → the `cargo` gates are unaffected (last green at the
cycle-6 close, T45). No Dockerfile change → the image still builds.

## D5 — screenshots + critical comparison

Captured from the **real `InlineSentenceBuilder`** rendered in the production
Tailwind chrome via a throwaway Vite harness (`web/devpreview/sentence.{html,tsx}`,
mocked 3-person roster, seeded `And[Paloma:is-present, people_count≥2,
face_recognition, date_range Jul-2024, media_type photo]`), driven headless with
Python Playwright (system chromium, `device_scale_factor=2`). Harness removed
after capture (mirrors the T47 teardown); recreate the two files to re-shoot.

- `cycle7-t48-pills-dark.png` — at rest, dark. Reads as one sentence:
  *"Include to album if Paloma is present and people count ≥ 2 and all faces
  must be recognized and taken between 2024-07-01 and 2024-07-31 and is a
  photo."* Each editable pill now carries the `▾` affordance; the live readout
  beneath matches verbatim.
- `cycle7-t48-editor-dark.png` — the people_count pill's inline editor open: a
  rounded dark popover labelled **People count** with the **≥ at least**
  operator `<select>` and the value field (2). Same chrome as the T47 person
  editor.
- `cycle7-t48-menu-light.png` — light mode, the **"+ condition"** menu open:
  the dashed trigger lights indigo and the popup lists all six leaf types
  (Person / People count (YOLO) / Face recognition / Date range / Location /
  Media type) under a **CONDITION** header.

**Critical comparison (design + immich-style-mirror):**
- ✓ Still reads like a sentence, not a stack of boxes; the new pills flow inline
  with the primary-coloured **and** connectors exactly like the person pill.
- ✓ Editors are dark-mode-first, `immich-primary`-accented, rounded — matching
  the style mirror and the T47 person editor.
- ✓ Every PRD predicate category now has an inline control on its pill except
  location (deliberately deferred to T50, which adds the numbered map blocks).
- ✓ The "+ condition" menu is discoverable and Immich-styled in both themes.
- Known T48 limits (by design, later tasks): location pill is read-only until
  T50; except clauses are T49; drag-drop is T51; the tree→sentence loader for
  existing rules + old-composer deletion is T52.

## Live verification

Deferred to **T53** (rebuild + redeploy + Chrome MCP against the deployed
build), per the cycle-7 plan. This D5 used the local production component, not
the live deploy.
