# POSTSHIP-T49 — Except-if clauses + inverse-fill serialization (D5 gate)

**Date:** 2026-05-29
**Scope:** Pure frontend. Adds the "Except if" clause UI to the cycle-7 inline
sentence builder (`web/src/components/blocks/InlineSentenceBuilder.tsx`). No
engine / schema / API changes — the builder still emits the same `yaml_source`.
Design contract: `docs/design/inline-sentence-builder.md` §5 (component tree)
and §6 (tree↔sentence mapping).

This records the mandatory **D5 UI quality gate** (cycle-7 LOCKED DECISION L5):
vitest-green is explicitly NOT sufficient — the UI was screenshotted from the
real production component (the actual `InlineSentenceBuilder`, real
`src/index.css`, real Tailwind tokens) and critically compared to the design +
`docs/design/immich-style-mirror.md` before commit.

## What shipped in T49

- **One `ClauseView` per `excepts[i]`**, each rendered on its own row with an
  amber left border, an "Except if" lead label, the clause's own all-of/any-of
  toggle (reused as-is — appears only at ≥2 pills), its inline condition pills,
  its own "+ condition" menu, and a "✕ clause" remove button.
- **"+ Except clause"** affordance below the primary clause (dashed pill).
- **Except-mutation handlers** mirroring the primary ones, keyed by clause
  index: `addExcept` / `removeExcept(i)` / `setExceptMode(i)` /
  `changeExceptPill(i,j)` / `removeExceptPill(i,j)` / `addExceptPill(i,kind)`.
  All route through `commit({...m, excepts})` so the model stays source of
  truth and the existing echo-guard preserves open editors.
- **Serialization was already in place and tested** (`sentenceModel.ts`:
  `baseMatch` emits `And[primary, Not(except1), …]`; Exclude fill wraps the
  whole base in a single outer `Not` — never `Not(Not(...))`). T49 only added
  the missing UI.
- **Empty-except guard** in `baseMatch`: a just-added `{pills:[]}` except (and
  an empty primary) are dropped from serialization so the tree never degenerates
  into `Not(And[])` / double-NOT. `normalizeTree` already strips this downstream;
  filtering in `baseMatch` keeps `sentenceToTree` self-consistent so the
  echo-guard never re-seeds and clears the operator's open (empty) clause.

## Screenshots (production component, device_scale_factor 2)

| File | What it proves |
| --- | --- |
| `cycle7-t49-excepts-dark.png` | At-rest dark. Primary `all of` clause (Paloma is present **and** Emeric may be present) + an "Except if" `any of` clause (Manon is present **or** people count ≥ 5), "✕ clause", "+ Except clause", and the live readout matching the operator's headline phrasing. |
| `cycle7-t49-excepts-light.png` | Same in light mode — white pills, indigo toggles, amber except rail; still reads as a sentence. |
| `cycle7-t49-except-editor-dark.png` | Clicking an **except** pill ("Manon is present") opens the same inline editor as a primary pill — mode dropdown ("is present") + reused `PersonPicker` (Selected: Manon). Confirms except conditions are fully editable, not read-only. |

Seed expr: `And[Person(must_include,paloma), Person(may_include,emeric),
Not(Or[Person(must_include,manon), PeopleCount(gte,5)])]` — loaded via
`treeToSentence`, proving the loader already round-trips the except shape.

## Critical comparison (D5)

- **Reads as a sentence?** Yes. "Include to album if Paloma is present and
  Emeric may be present. Except if Manon is present or people count ≥ 5." The
  except clause is visually subordinate (indented, amber rail, "Except if"
  lead) but composes into the same readout sentence.
- **Immich style?** Dark-first; indigo `immich-primary` for active segmented
  toggles; `ui-border` / `immich-dark-gray` pills; `ui-muted` secondary text;
  amber accent for the "except" exception, consistent with the fallback notice
  amber. Matches the style mirror.
- **Not generic / not cluttered?** Each except is one compact row; the toggle
  hides until a clause has ≥2 pills; the connectors ("and"/"or") are inline
  words, not chips. No stacked-card density of the old builder.

## Gates (all green)

- `npm run typecheck` — clean
- `npm run lint` — clean
- `npx vitest run` — **323** tests across 28 files (+5 vs T48's 318):
  single-condition primary → bare leaf; include+1 except; include+2 excepts;
  exclude+except (no double-NOT); empty-except no-op + removable. Each
  structural case round-trips `formStateToYamlV2`→`yamlToFormStateV2`.
- `npm run build` — 194.16 kB main (+1.62 kB vs T48)
- `cargo fmt --all --check` — clean (no Rust changes this task)
