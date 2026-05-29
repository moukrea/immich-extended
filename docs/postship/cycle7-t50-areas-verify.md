# POSTSHIP cycle 7 — T50 D5 verify: numbered geo Area blocks

**Task.** Wire the inline `taken in Area N` location pill to a numbered
`MapPicker` block rendered below the sentence; multiple areas coexist and
renumber on add/remove (per `docs/design/inline-sentence-builder.md` §3.1, L3).

**Build/test gates (all green):**
- `npm run typecheck` — clean
- `npm run lint` — 0w / 0e
- `npm test -- --run` — **324 vitests** across 28 files (T49 was 323; −1 placeholder, +2 area tests)
- `npm run build` — main **198.15 kB** / 61.26 kB gzip + lazy MapPicker 1054 kB (unchanged chunk; +3.99 kB main vs T49 for the area blocks)

New tests in `inlineSentenceBuilder.test.tsx`:
- a location pill reads `taken in Area 1`, is not disabled, and renders a
  numbered map block + the Areas legend in the readout;
- two areas → numbered `Area 1`/`Area 2` pills + two map blocks; editing Area 2's
  radius via its (mocked) map updates the right leaf (`And[loc(60), loc(123)]`);
  removing Area 1 renumbers the survivor to Area 1 (`loc(123)` bare leaf) and
  drops `area-block-2`.

## D5 screenshots (Chrome MCP / swiftshader WebGL, device_scale_factor=2)

Seed: `And[ Person{paloma,must_include}, Location{Paris,60km}, Not(Location{Nice,25km}) ]`
→ primary `Paloma is present and taken in Area 1`, except `taken in Area 2`.

- `cycle7-t50-areas-dark.png` — dark-mode-first. Inline sentence with the
  `taken in Area 1`/`Area 2` pills (📍 affordance), the `Except if` clause, the
  readout line with the Areas legend, and **two numbered map blocks below** with
  real OSM tiles.
- `cycle7-t50-areas-light.png` — light theme parity; MapPicker controls legible.
- `cycle7-t50-pill-focus-dark.png` — clicking `taken in Area 1` flashes an
  `immich-primary` ring on the Area 1 block (the pill→map link).

### Critical comparison vs design + immich-style-mirror

- **Reads as a sentence** ✓ — pills flow inline with `and` / `Except if`
  connectives; numbered areas read as plain language (`taken in Area 1`).
- **Numbered maps below the sentence** ✓ — each area block has a primary-color
  number badge + `Area N` label + reused `MapPicker`; renumber proven by test.
- **Pill ↔ block link** ✓ — the location pill is clickable (not a dead disabled
  pill); it scrolls to and flashes its block.
- **Dark-first immich palette** ✓ — Include/all-of toggles use `immich-primary`;
  area cards use `dark:bg-immich-dark-gray` / `border-ui-border`.

### Known limitation (in scope)

`MapPicker`'s own radius label + helper text use M4-era `text-slate-700`, so they
read dim on the dark area card. cycle-7 ABSOLUTE rules require REUSING
`MapPicker` (not rebuilding it), and this dimness exists anywhere MapPicker is
mounted — it is not a T50 regression. The area-block chrome I added (badge,
label, card, flash ring) is dark-mode-correct.

*Verified 2026-05-29 by POSTSHIP-T50. Harness `web/devpreview/` removed after capture.*
