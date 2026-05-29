# POSTSHIP-T47 — Inline sentence builder shell + person mode dropdown (D5 gate)

**Date:** 2026-05-29
**Scope:** Pure frontend. New `web/src/lib/sentenceModel.ts` (pure model + tree
mapping), `leafSentence` added to `web/src/lib/phrases.ts`, new
`web/src/components/blocks/InlineSentenceBuilder.tsx`, swapped in for
`BlockTreeEditor` at `web/src/pages/rules/RuleBuilderV2.tsx`. No engine / schema
/ API changes (the builder emits the same `yaml_source`). Design contract:
`docs/design/inline-sentence-builder.md` §3–§6, §9.

This records the mandatory **D5 UI quality gate** (cycle-7 LOCKED DECISION L5):
vitest-green is explicitly NOT sufficient — the UI was screenshotted from the
real production component and critically compared to the design + the Immich
style mirror before commit.

## What shipped in T47

- **`sentenceModel.ts`** — `SentenceModel { fill, primary, excepts[] }`;
  `sentenceToTree` (single pill → bare leaf; clause all→And / any→Or; excepts →
  `Not(...)` under an `And`; Exclude fill → outer `Not`; never `Not(Not(...))`)
  and the conservative loader `treeToSentence` (returns `null` for Or-of-Ands,
  `Person{includes}`, double-NOT, and any non-flat arrangement → Advanced-YAML
  fallback, never corrupts). Plus `sentenceReadout` (the live full-sentence
  line, with numbered "Area N" legend for location leaves).
- **`phrases.ts` `leafSentence`** — at-rest natural language for every leaf:
  person `is present` / `may be present` / `is not present`; date
  `between/after/before`; location `taken in Area N`; etc.
- **`InlineSentenceBuilder.tsx`** — `LeadToggle` (Include/Exclude, L1), the
  primary `ClauseView` with the `all of` / `any of` toggle (L2), inline person
  `ConditionPill` rendering "<name> <mode>" at rest with a click-to-open editor
  carrying the **mode dropdown {is present, may be present, is not present}** +
  the reused `PersonPicker`, a "+ condition" affordance, the always-on
  `ReadoutLine`, and the fallback notice. Model is the source of truth; an echo
  guard skips re-seeding on our own `onChange` so open editors survive edits.
- **`RuleBuilderV2.tsx`** — `<BlockTreeEditor>` → `<InlineSentenceBuilder>`
  (Advanced YAML panel, export/import, lifecycle all kept;
  `onRequiresAdvanced` auto-expands the YAML panel on fallback).

THE bug fixed (T47 headline): person mode was read-only with a hard
`must_include` default, so a *second* "may be present" person was impossible.
The inline mode dropdown makes it possible — see the screenshots.

## Automated gates (web)

`cd web`:
- `npm run typecheck` ✓
- `npm run lint` ✓ (0 warnings / 0 errors)
- `npm test -- --run` ✓ — **313 vitests across 28 files** (added
  `lib/__tests__/sentenceModel.test.ts` 25 + `blocks/__tests__/inlineSentenceBuilder.test.tsx` 3;
  rewrote `pages/rules/__tests__/ruleBuilderV2.test.tsx` 12 onto the new builder).
- `npm run build` ✓ — main `185.03 kB / 57.87 kB gzip` (the old block builder's
  lazy MapPicker chunk drops out — no map UI until T50 reintroduces it).

No Rust files changed this task, so the `cargo` gates are unaffected (last green
at the cycle-6 close, T45). No Dockerfile change → the image still builds.

## D5 — screenshots + critical comparison

Captured from the **real `InlineSentenceBuilder`** rendered in the Tailwind
chrome via a throwaway Vite harness (`web/devpreview/sentence.{html,tsx}`,
mocked 3-person roster, seeded with the marquee case `And[Paloma:is-present,
Emeric:may-be-present]`), driven headless with Python Playwright (system
chromium, `device_scale_factor=2`). Harness removed after capture (mirrors the
T35 devpreview teardown); recreate the two files to re-shoot.

- `cycle7-t47-builder-dark.png` — at rest. Reads as one sentence:
  *"Include to album if Paloma is present and Emeric may be present."* Pills flow
  inline with the primary-colored **and** connector; the live readout beneath
  matches; the segmented **Include/Exclude** and **all of/any of** toggles fill
  with `immich-primary` when active.
- `cycle7-t47-builder-editor-dark.png` — the Emeric pill's inline editor open:
  the **Condition** mode dropdown reads **"may be present"** (options: is present
  / may be present / is not present) above the `PersonPicker` (Emeric ringed as
  selected). This is the bug fix — a *second* may-be-present person.
- `cycle7-t47-builder-editor-light.png` — same, light mode: white card, indigo
  active toggles, the dashed "+ condition" affordance clearly legible.

**Critical comparison (design + immich-style-mirror):**
- ✓ Reads like a sentence, not a vertical stack of boxes (the operator's
  core complaint about the old builder).
- ✓ Dark-mode-first; `immich-primary` accents; rounded filled controls;
  Overpass type — consistent with the mirror.
- ✓ A second "may be present" person is addable (regression target).
- ✓ Include/Exclude and all/any toggles are obvious; the live readout is legible
  and tracks the pills.
- Known T47 limits (by design, later tasks): other leaf editors are T48; except
  clauses T49; numbered geo Area maps T50; drag-drop T51; the tree→sentence
  loader for existing rules + old-composer deletion T52.

## Live verification

Deferred to **T53** (rebuild + redeploy + Chrome MCP against the deployed build),
per the cycle-7 plan. This D5 used the local production component, not the live
deploy.
