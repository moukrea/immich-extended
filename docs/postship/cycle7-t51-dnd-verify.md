# POSTSHIP cycle 7 — T51 D5 verify: drag-and-drop pill reordering

**Task.** Make sentence pills reorderable within a clause and movable across
clauses (primary ↔ except) via drag-and-drop, with a keyboard fallback, per
`docs/design/inline-sentence-builder.md` §8 (L?/T51). The `SentenceModel` is the
source of truth, so the move operates on the clause `pills` arrays directly — NOT
the path-addressed tree `treeOps` (that serves the retired tree builder).

## What shipped

- **Pure move logic in `sentenceModel.ts`** (independently unit-tested):
  - `PillLoc` (reuses the `AreaRef` primary/except discriminator) + `pillLocKey`.
  - `movePill(model, from, to)` — splice a pill out of its source clause and into
    the target clause at the drop gap; within-clause same-array adjustment;
    immutable; out-of-range = no-op.
  - `movePillStep(model, loc, "earlier"|"later")` — keyboard nudge that steps
    through the document-order pill sequence, crossing clause boundaries (last
    pill of a clause → head of the next; first pill → tail of the previous), so
    the grip's arrow keys cover BOTH "move within" and "move to clause".
- **`InlineSentenceBuilder.tsx` wiring:**
  - `ConditionPill` gets a `data-drag-handle` grip (⠿, opacity-30 → full on
    hover/focus), the root span is `draggable` only while the grip is held (so a
    drag never starts from inside the inline editor popup), and it is a drop
    target (`dragover`/`drop`). Source dims (`opacity-50`); the hovered target
    gets an `immich-primary` ring.
  - The "+ condition" affordance doubles as the **clause-end drop zone** (append).
  - Single `hoverKey` signal (set on `dragenter`, cleared at drag end/drop) so
    exactly one target highlights and it follows the cursor with no flicker.
  - Keyboard: ArrowLeft/Up = earlier, ArrowRight/Down = later, on the grip button.
- Cross-clause moves re-serialize immediately — a pill dragged primary→except is
  now under that except's `Not(...)`, so the readout + YAML change on drop.

## Build/test gates (all green)

- `npm run typecheck` — clean
- `npm run lint` — 0w / 0e
- `npm test -- --run` — **337 vitests** across 28 files (T50 was 324; +13:
  10 pure `movePill`/`movePillStep` cases in `sentenceModel.test.ts`, 3 DOM drag
  cases in `inlineSentenceBuilder.test.tsx`).
- `npm run build` — main **202.37 kB** / 62.59 kB gzip + lazy MapPicker 1054 kB
  (+4.22 kB main vs T50 for the DnD wiring).
- `cargo fmt --all --check` — clean (no Rust changes this cycle).

New tests:
- `sentenceModel.test.ts`: within-clause reorder permutes order + round-trips;
  no input mutation; drop-on-self no-op; primary→except negates the pill in the
  tree (`And[A, Not(B)]`); append at clause length; `movePillStep` later/earlier
  within a clause; step across the primary↔except boundary; no-op at the ends.
- `inlineSentenceBuilder.test.tsx`: drag reorders within a clause (`dragStart` +
  `drop`, readout updates); dragging a primary pill onto an except's end zone
  negates it (`And[Paloma, Not(And[count, Emeric])]`); ArrowRight on the grip
  reorders within the clause.

## D5 screenshots (Chrome / chromium swiftshader, device_scale_factor=2)

Seed: `And[ Person{paloma,must_include}, Person{emeric,may_include},
Location{Paris,60km}, Not(Person{manon,must_include}) ]` → primary
"Paloma is present and Emeric may be present and taken in Area 1", except
"Manon is present".

- `cycle7-t51-grips-dark.png` — at-rest sentence with the ⠿ grip on every pill
  (forced fully visible for the static shot; in product they fade in on
  hover/focus). Still reads as a sentence; `and` connectives + `Except if` rail.
- `cycle7-t51-dragging-dark.png` — mid-drag: "Emeric may be present" (drag
  source) is dimmed, "Paloma is present" (drop target) wears the `immich-primary`
  ring. Drag-in-progress feedback proven.
- `cycle7-t51-dragging-light.png` — light-theme parity of the mid-drag state.

### Critical comparison vs design + immich-style-mirror

- **Reads as a sentence** ✓ — the grip is a small, low-contrast affordance to the
  left of each pill; the inline phrasing + `and`/`Except if` connectives are
  unchanged from T47–T50.
- **Drag affordance + feedback** ✓ — grip handle (mirrors the retired
  `PillCard`'s `data-drag-handle`), dimmed source, ringed drop target, clause-end
  drop zone on "+ condition".
- **Cross-clause = semantic change** ✓ — moving primary→except wraps the pill in
  `Not(...)`; unit + DOM tests assert the tree and readout update.
- **A11y fallback** ✓ — grip is a focusable button; arrow keys move within and
  across clauses; covered by a vitest.
- **Dark-first immich palette** ✓ — ring/toggles use `immich-primary`; pills use
  `dark:bg-immich-dark-gray` / `border-ui-border`.

### Known limitation (in scope, not a T51 regression)

The Area 1 `MapPicker` renders an empty bordered box in the harness (MapLibre
needs WebGL + network OSM tiles, stubbed offline). The DnD work is entirely above
the map block; tiles render live in the deployed build (proven in T50). T53
re-verifies maps against the real deployment.

*Verified 2026-05-29 by POSTSHIP-T51. Harness `web/devpreview/` removed after capture.*
