# Inline natural-language sentence rule builder

> **Task.** POSTSHIP cycle 7 (T46–T53). Replace the stacked block builder
> (`BlockTreeEditor` + pill/group cards) with an **inline sentence composer**:
> the rule reads as one English sentence —
> *"Include to album if Paloma is present and Emeric may be present. Except if
> Manon is present."* — with each pill an at-rest phrase that reveals an inline
> editor on click, and a live full-sentence readout always shown.
>
> **Scope.** Pure **frontend** rebuild. **No engine / schema / API changes.**
> The `MatchExpr` tree already expresses every shape this UI needs (verified
> against `crates/engine/src/rule/match_expr.rs` + `validator.rs`); the builder
> emits the same `yaml_source` the API already accepts. Backend `cargo` gates
> stay green with zero Rust edits.
>
> **Supersedes** `docs/design/dnd-block-builder.md` for the *builder UI* only.
> The module docs in that file for `phrases.ts`, `treeOps.ts`, and `matchTree.ts`
> remain accurate — those modules are **reused**, not rebuilt.

This doc is the contract for T47–T53. The **LOCKED DECISIONS (L1–L5)** in
`.ralph/TASKS.md` "CYCLE 7" override any conflicting prose here.

---

## 1. What we're building (and not)

The inline sentence builder is **not** a general tree editor. It edits the
*common* rule shape — a primary clause plus zero or more "except if" clauses,
each clause being a flat all/any list of conditions — which covers every rule
the operator actually writes. Genuinely nested logic (an OR of ANDs, e.g. the
cycle-4 Example D) does **not** fit the sentence shape and **falls back to the
Advanced YAML panel** (§7). This is deliberate and consistent with **L2** ("no
inline per-connector grouping; mixed logic via except-clauses + person modes").

The previous builder failed the operator on two counts this rebuild fixes:

1. **The marquee bug (T47).** Person pills render their mode read-only
   (`PillCard.tsx` `personVerb()` is plain text; `defaults.ts:32` hard-defaults
   `must_include`), so you cannot add a *second* "may be present" person. The new
   person pill carries an inline **mode dropdown** `{is present, may be present,
   is not present}`.
2. **It reads as stacked boxes, not a sentence.** The new layout flows pills
   inline with connective words between them and a readout line beneath.

---

## 2. The backend contract we bind to (unchanged — do not edit)

`MatchExpr` (Rust `match_expr.rs`, TS mirror `web/src/lib/matchTree.ts`):

```
MatchExpr = And(children≥0) | Or(children≥0) | Not(child) | Leaf
MatchLeaf = Person{ mode, person_id }
          | PeopleCount{ op, value }
          | FaceRecognition{ allow_unrecognized, yolo_count_check }
          | DateRange{ from?, to? }
          | Location{ center:[lat,lng], radius_km }
          | MediaType{ types:[photo|video] }
PersonMode    = must_include | may_include | must_exclude | includes
PeopleCountOp = eq | ne | lt | lte | gt | gte
```

**Validator rules that constrain what the builder may emit**
(`crates/engine/src/rule/validator.rs`):

| Rule | Error slug | Consequence for the builder |
|---|---|---|
| depth ≤ 8 | `match_tree_too_deep` | Non-issue: sentence trees max out at depth ~5 (§6.4). |
| And/Or need **≥ 2** children | `empty_match` / `redundant_group` | A clause with **one** pill **must** serialize as a **bare leaf**, never `And[leaf]`/`Or[leaf]`. |
| `Not` child may not be `Not` | `double_not` | Never emit `Not(Not(...))`. Exclude-fill wraps a non-Not baseMatch, so it's safe. |
| `Person{includes}` only as the **direct** child of a `Not` | `includes_outside_not` | The builder **never emits `includes`** — "is not present" emits a bare `Person{must_exclude}` (slug-stable, matches `From<&MatchSpec>`). |
| `person_id` non-empty + owned by the rule user | `empty_person_id` / `foreign_person_id` | Person pill disables Save until a person is picked (server re-checks ownership). |
| DateRange not both-null, `from ≤ to` | `empty_date_range` / `invalid_date_range` | Date pill validates client-side; bad ranges block Save. |
| Location bounds (lat/lng/radius) | `invalid_location` | MapPicker already clamps. |

**Serialization** is byte-for-byte via the existing
`serializeMatchExpr` / `parseMatchExpr` (`matchTree.ts`) and
`formStateToYamlV2` / `yamlToFormStateV2` (`ruleYamlV2.ts`). The builder produces
a `MatchExpr`; RuleBuilderV2 already turns that into `yaml_source`.

---

## 3. The sentence model (UI state)

The builder's **source of truth is a `SentenceModel`**, derived from the rule's
`MatchExpr` on load and re-serialized to a `MatchExpr` on every edit. (Rationale
in §5.)

```ts
type Fill = "include" | "exclude";          // L1 lead toggle
type ClauseMode = "all" | "any";            // L2 per-clause AND/OR

interface Clause {
  mode: ClauseMode;
  pills: MatchLeaf[];                        // ordered; each pill is one leaf
}

interface SentenceModel {
  fill: Fill;
  primary: Clause;                           // the "if" clause
  excepts: Clause[];                         // zero or more "except if" clauses
  // Geo "Area N" numbering (L3) is *derived*, not stored — see §3.1.
}
```

A **pill** is exactly one `MatchLeaf`. There is no separate "group pill": nesting
is expressed by clauses (primary + excepts) and per-clause all/any, never by
inline grouping.

### 3.1 Geo areas are a derived view (L3)

A `Location` leaf renders inline as **"taken in Area N"**. `N` is assigned by
**document order** of every `Location` pill across `primary.pills` then each
`excepts[i].pills`. Below the sentence, one numbered **`MapPicker` block** per
`Location` pill (reusing `web/src/components/MapPicker.tsx`) shows/edits its
`center` + `radius_km`. Adding a location pill spawns a new area block; removing
one **renumbers** the rest. Areas are computed each render from the model —
there is no separate `areas` field to keep in sync.

---

## 4. At-rest phrasing (L4)

Every pill shows plain-language text at rest and reveals its editor on click. The
wording source stays `web/src/lib/phrases.ts` (`leafPhrase` / `leafPhraseText`);
T47/T48 extend it. Canonical phrasing table:

| Leaf | Mode / fields | At-rest phrase |
|---|---|---|
| Person | `must_include` | `Paloma is present` |
| Person | `may_include` | `Emeric may be present` |
| Person | `must_exclude` | `Manon is not present`  ⟵ **changed** from the old `never Manon` strip wording |
| Person | `includes` | *(never emitted by the builder; only seen in fallback-only trees)* |
| PeopleCount | `op`,`value` | `people count = 1`, `people count ≥ 2` (symbol from `OP_SYMBOL`) |
| FaceRecognition | `!allow_unrecognized` | `all faces must be recognized` |
| FaceRecognition | `!allow_unrecognized` + `yolo_count_check` | `all faces must be recognized · reject extra humans (YOLO)` |
| FaceRecognition | `allow_unrecognized` + `yolo_count_check` | `no unidentified extra humans (YOLO)` |
| FaceRecognition | `allow_unrecognized` only | `unrecognized faces allowed` |
| DateRange | from+to | `taken between 2024-07-15 and 2024-07-22` |
| DateRange | from only | `taken after 2024-07-15` |
| DateRange | to only | `taken before 2024-07-22` |
| Location | — | `taken in Area N` (the lat/lng/radius live in the numbered map block) |
| MediaType | `[photo]` / `[video]` / both | `is a photo` / `is a video` / `is a photo or video` |

**Person mode dropdown** (the T47 fix) maps exactly: `is present → must_include`,
`may be present → may_include`, `is not present → must_exclude`.

### 4.1 Live readout line (L4)

A read-only sentence is always rendered (aria-live polite). Assembly:

```
<Lead> <primary clause>. [Except if <except clause>.]…  [Areas: 1 = …; 2 = …]
```

- **Lead**: `Include to album if` / `Exclude from album if`.
- **Clause**: pill phrases joined by `" and "` (mode `all`) or `" or "`
  (mode `any`). A single pill has no connector.
- **Excepts**: each prefixed `Except if ` and terminated `.`.
- **Areas legend** (optional, only if ≥1 location pill): `Areas: N = within R km
  of (lat, lng)` so the numbered pills resolve to coordinates in one glance.

Example readout:
*"Include to album if Paloma is present and Emeric may be present. Except if
Manon is present."*

---

## 5. Component tree

`InlineSentenceBuilder` replaces the `<BlockTreeEditor expr onChange/>` call in
`RuleBuilderV2.tsx` (currently `RuleBuilderV2.tsx:517-519`). Its props are
unchanged from the slot it fills:

```tsx
<PeopleProvider>
  <InlineSentenceBuilder expr={expr()} onChange={mutateExpr} />
</PeopleProvider>
```

```
InlineSentenceBuilder(props: { expr: MatchExpr; onChange: (e: MatchExpr) => void })
│  // internal signal: model: SentenceModel | null  (null ⇒ fallback, §7)
│  // seed: createEffect(on(() => props.expr, e => setModel(treeToSentence(e))))
│  // commit: on any model edit → onChange(normalizeTree(sentenceToTree(model)))
│
├─ <FallbackNotice/>            // when model === null: "this rule uses advanced
│                               //   logic — edit it in the Advanced YAML panel"
│
└─ when model !== null:
   ├─ <LeadToggle/>             // segmented Include / Exclude (L1) → model.fill
   ├─ <ClauseView clause=primary primary />
   │   ├─ <ClauseModeToggle/>   // all / any segmented (L2) → clause.mode
   │   ├─ For pills → <ConditionPill leaf onChange onRemove dragHandle/>
   │   └─ <AddConditionButton/> // "+ condition" menu of the 6 leaf types
   ├─ For model.excepts → <ExceptClause>
   │   ├─ label "Except if"
   │   ├─ <ClauseView clause=except/>      // same as primary, its own all/any
   │   └─ <RemoveClauseButton/>            // "✕ clause"
   ├─ <AddExceptButton/>        // "+ Except clause"
   ├─ <ReadoutLine model/>      // §4.1, always visible, aria-live
   └─ <AreaBlocks model/>       // §3.1, one numbered <MapPicker> per Location pill
```

**`ConditionPill`** renders the at-rest phrase from `phrases.ts` and, on click,
reveals its inline editor in place (the controls mount once and read
`props.leaf` reactively so typing keeps focus — same technique as today's
`PillCard`). Editors by leaf type:

- **person**: `PersonPicker` (reused) **+ mode `<select>`** `{is present, may be
  present, is not present}` — *the bug fix*.
- **people_count**: op `<select>` + number input.
- **face_recognition**: two checkboxes (recognize-all, reject-extra-YOLO).
- **date_range**: two `<input type=date>` (both shown so an empty range stays
  editable).
- **location**: read-only "Area N"; its map is the linked `AreaBlock` below.
- **media_type**: `<select>` `{photo, video, photo or video}`.

**Why `SentenceModel` is the source of truth (not the tree).** The sentence is a
flatter structure than `MatchExpr`; editing clause arrays directly (add/remove/
reorder a pill, flip a clause toggle) is trivial and unambiguous, whereas
path-addressed tree edits are awkward for a flat sentence. We round-trip:
`props.expr → treeToSentence → (edit model) → sentenceToTree → onChange`. The
`createEffect` re-seeds the model when `props.expr` changes from *outside* (e.g.
the user typing in the Advanced YAML panel), so the two editors stay coherent.

---

## 6. Tree ↔ sentence mapping

### 6.1 sentence → tree (`sentenceToTree`)

```
clauseExpr(clause):
    leaves = clause.pills            // each pill → its MatchLeaf
    if leaves.length == 1:  return leaves[0]                 // bare leaf (≥2 rule)
    return clause.mode == "all" ? And(leaves) : Or(leaves)

baseMatch(model):
    p = clauseExpr(model.primary)
    if model.excepts is empty:  return p
    return And([ p, ...model.excepts.map(c => Not(clauseExpr(c))) ])

sentenceToTree(model):
    b = baseMatch(model)
    return model.fill == "include" ? b : Not(b)             // L1
```

Then `normalizeTree(...)` (existing `treeOps.ts`) drops any empties before
`onChange`. **Never emit `Not(Not(...))`** — `baseMatch` is always a leaf/And/Or,
never a `Not`, so `Not(baseMatch)` is single-level. ✓

### 6.2 tree → sentence (`treeToSentence`) — the loader (T52)

Conservative structural matcher. Returns `null` ⇒ **Advanced-YAML fallback**
(§7). `pillLeaf` = any `MatchLeaf` **except** `Person{includes}` (which the
builder never emits; its only source is hand-written YAML → fall back).

```
treeToSentence(expr):
    (fill, base) = expr is Not(child) ? ("exclude", child) : ("include", expr)
    if base is Not(...):  return null                       // double-not ⇒ fallback
    split = splitBase(base)
    if split is null:  return null
    return { fill, primary: split.primary, excepts: split.excepts }

splitBase(base):
    if base is pillLeaf:                 return { primary:{all,[base]}, excepts:[] }
    if base is Or(ch) and all pillLeaf:  return { primary:{any, ch},   excepts:[] }
    if base is And(ch):
        nots    = ch.filter(c => c is Not)
        nonNots = ch.filter(c => c is not Not)
        excepts = nots.map(n => clauseFromExpr(n.child)); if any null → null
        if nonNots is empty:  return null
        if nonNots == [Or(or)] and all or are pillLeaf:
            primary = { any, or }
        else if all nonNots are pillLeaf:
            primary = { all, nonNots }
        else:
            return null                  // a non-Not group we can't flatten ⇒ fallback
        return { primary, excepts }
    return null

clauseFromExpr(e):                       // for an except's Not(child)
    if e is pillLeaf:                 return { all, [e] }
    if e is And(ch) and all pillLeaf:  return { all, ch }
    if e is Or(ch)  and all pillLeaf:  return { any, ch }
    return null
```

### 6.3 Worked examples

| Sentence | `MatchExpr` (canonical) | Round-trips? |
|---|---|---|
| `Include … if Paloma is present.` (legacy `beba1580`) | `Person{must_include,paloma}` | ✓ bare leaf |
| `Include … if Paloma is present and Emeric may be present. Except if Manon is present.` | `And[ P{mi,paloma}, P{may,emeric}, Not(P{mi,manon}) ]` | ✓ all + 1 except |
| `Exclude … if (taken in Area 1 or is a video).` | `Not(Or[ Location{…}, MediaType{[video]} ])` | ✓ exclude + any + geo |
| `Include … if people count ≥ 2.` | `PeopleCount{gte,2}` | ✓ bare leaf |
| Example D: `( Paloma AND count=1 ) OR ( Paloma AND Emeric AND count≥2 ) MUST EXCLUDE Manon` | `And[ Or[ And[…], And[…] ], Not(P{includes,manon}) ]` | ✗ → **fallback** (Or-of-Ands isn't a flat clause) |

The fallback in the last row is correct behavior, not a defect: the inline
builder handles flat clauses; deeply nested mixed logic is authored as YAML.

### 6.4 Depth budget

Worst case (exclude fill, any-primary, an And/Or except):
`Not(1) → And(2) → Or|Not(3) → And|Or(4) → leaf(5)` = **depth 5 ≤ 8**. The
sentence shape can never bust the cap.

---

## 7. Advanced-YAML fallback (never corrupt — ABSOLUTE)

When `treeToSentence(expr)` returns `null`, the builder:

1. Hides the sentence editor and renders a **`<FallbackNotice>`**: *"This rule
   uses advanced logic that the sentence builder can't show. Edit it in the
   Advanced (YAML) panel below."* — and auto-expands the Advanced panel
   (`setShowAdvanced(true)` is already wired in `RuleBuilderV2.tsx`).
2. **Touches nothing.** It does **not** call `onChange`. The rule's `expr` /
   `yaml_source` pass through verbatim. This is the cycle-7 ABSOLUTE rule.
3. Re-attempts conversion whenever the YAML panel produces a new `expr` (the
   re-seed effect, §5) — so editing the YAML back into a fittable shape
   re-enables the sentence editor live.

Fallback triggers: Or-of-Ands / And-of-Ands nesting, a `Not` wrapping a nested
group, `Person{includes}`, or any non-canonical arrangement (§6.2 `null` paths).

---

## 8. Drag-and-drop (T51)

Pills are reorderable within a clause and movable across clauses (primary ↔
except). Because `SentenceModel` is the source of truth (§5), DnD operates on the
**clause `pills` arrays directly**:

- **Within-clause reorder**: splice within `clause.pills`. Semantically a no-op
  (AND/OR commute) but preserves the operator's reading order; re-serialize so
  YAML reflects it.
- **Cross-clause move**: splice out of the source clause, splice into the target
  at the drop index. This **changes semantics** (a pill moving primary→except is
  now negated) — the readout + YAML update immediately so the change is visible.
- **Keyboard fallback**: ↑/↓ move-within and a "move to clause" affordance for
  a11y.

Mechanics: HTML5 drag (`draggable` pills, `dragover`/`drop` on clause pill-lists)
with a `data-drag-handle` grip, mirroring today's `PillCard`. The path-addressed
`treeOps.moveNode` remains the equivalent operation **if** a later refactor makes
the tree the source of truth; for the sentence model, array splices are simpler
and sufficient. Whichever is used, the post-move tree must still pass
`normalizeTree` + stay ≤ depth 8 (it always does — moves don't add nesting).

---

## 9. D5 UI quality gate (L5 — mandatory every UI task)

For **each** of T47, T48, T50, T51, T53: `cd web && npm run build`, open the
bundle in Chrome MCP, screenshot, and **critically compare** to this design +
`docs/design/immich-style-mirror.md` (dark-mode-first; `rounded-xl` filled
inputs; `immich-primary` accents; Overpass type). Iterate if it reads generic,
cluttered, or *not like a sentence* **before** committing. Save the screenshot to
`docs/postship/`. **vitest-green is NOT sufficient** — there is no operator review
gate; this self-check is the only quality control.

Sentence-specific things the screenshot must confirm: pills flow inline with
real connective words (not a vertical stack of boxes); the readout line is
legible and matches the pills; a second "may be present" person is addable; the
Include/Exclude and all/any toggles are obvious; area maps are numbered and sit
below the sentence.

---

## 10. File inventory

**Reuse (do not rebuild):**
- `web/src/components/MapPicker.tsx` — area map blocks.
- `web/src/components/blocks/PersonPicker.tsx` — person editor (single-person).
- `web/src/components/PeopleContext.tsx` — one `/api/v1/me/people` fetch shared.
- `web/src/lib/matchTree.ts` — TS `MatchExpr`, constructors, `serializeMatchExpr`,
  `parseMatchExpr`, `legacyMatchSpecToTree`.
- `web/src/lib/ruleYamlV2.ts` — `formStateToYamlV2` / `yamlToFormStateV2`.
- `web/src/lib/treeOps.ts` — `normalizeTree` (serialization boundary); `moveNode`
  et al. available if needed.
- `web/src/lib/phrases.ts` — wording; **extend** (location → "Area N", person
  `must_exclude` → "is not present", date "between"/"after"/"before").

**Add:**
- `web/src/components/blocks/InlineSentenceBuilder.tsx` (+ `LeadToggle`,
  `ClauseView`, `ConditionPill`, `ExceptClause`, `ReadoutLine`, `AreaBlocks`,
  `FallbackNotice` — co-located or split as the implementer prefers).
- `web/src/lib/sentenceModel.ts` — `SentenceModel` types, `treeToSentence`,
  `sentenceToTree` (pure, unit-tested independent of rendering).

**Replace:** the composer block in `web/src/pages/rules/RuleBuilderV2.tsx`
(`RuleBuilderV2.tsx:511-520`) — swap `<BlockTreeEditor>` for
`<InlineSentenceBuilder>`. Keep everything else (name/target/lifecycle/Advanced
YAML panel/export-import) intact.

**Delete (after T52, no orphan imports):**
`web/src/components/blocks/{BlockTreeEditor,NodeView,GroupCard,PillCard,SelectionBar,ExcludeStrip}.tsx`
and their tests under `web/src/components/blocks/__tests__/`
(`nodeView`, `pillCard`, `selectionBar`, `excludeStrip`, `partition`).
`AddBlockDropdown.tsx` and `defaults.ts` are **not** in the locked delete list —
the new "+ condition" menu may reuse them; if they end up unused after the
rebuild, delete them too (no orphans) — otherwise keep.

---

## 11. Test plan (drives T47–T53 vitest + T53 live)

Pure `sentenceModel.ts` tests (no DOM):
- `sentenceToTree`: single pill → bare leaf; all/any ≥2 → And/Or; include+1
  except → `And[p, Not(e)]`; include+2 excepts; exclude → outer `Not`;
  exclude+except; never `Not(Not)`; depth ≤ 8.
- `treeToSentence` (loader): inverse of each above; the operator's 2 Paloma
  rules (legacy `beba1580` + `714dce95`); a deliberately non-fitting tree
  (Example D) → `null` (fallback); `Person{includes}` → `null`.
- Round-trip: `treeToSentence(sentenceToTree(m)) ≅ m` for all canonical shapes.

Component tests (Solid Testing Library):
- Person pill: add a **second** `may_include` person (regression for the bug);
  changing the mode dropdown updates the emitted YAML.
- Each leaf pill renders its at-rest phrase and edits round-trip to YAML.
- Add/remove except clause; per-clause all/any toggle changes the connector.
- Add two areas → numbered maps below; remove the first → renumber.
- DnD reorder within a clause and move across clauses.
- Fallback: loading Example D shows the notice + Advanced panel and does not
  mutate the YAML.

T53 live (Chrome MCP, deployed): compose the operator's headline example,
confirm readout + YAML; add a second "may be present" person; toggle
Include→Exclude (inverse-fill); add two geo areas; save + reload round-trips;
the two existing rules open or fall back cleanly (never corrupt).

---

*Authored 2026-05-29 by POSTSHIP-T46. Sources read: `match_expr.rs`,
`validator.rs:194-303`, `matchTree.ts`, `ruleYamlV2.ts`, `treeOps.ts`,
`phrases.ts`, `PillCard.tsx`, `PersonPicker.tsx`, `RuleBuilderV2.tsx`,
`defaults.ts`, `immich-style-mirror.md`.*
