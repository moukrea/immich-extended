# Block-based rule builder — schema, evaluator, UX spec

**Status**: DESIGN (POSTSHIP-T15). No code shipped yet.
**Gates**: POSTSHIP-T18 (parser/validator), T19 (evaluator), T20 (UI).
**Owner**: worker, awaiting operator review of the open questions in §10 before T18 starts.

This document is the contract for the block-based rule builder. T18/T19/T20 implementers MUST NOT re-decide the shape; they may only refine details flagged "open" in §10. The PRD (specifically §6 — Match DSL, §11 — WebUI rules page, Appendix A — example rules) and `CLAUDE.md` "OPERATOR DIRECTIVES (2026-05-27, POSTSHIP cycle 4) §3" are the source authority; anywhere this doc conflicts with them, those win.

---

## 1. Goal

Replace the linear-form `RuleBuilder.tsx` (six sections in fixed order: Name / Target album / Date / Location / People / Media) with a **block-tree sentence composer** that reads in plain English:

> Include media to album **\<X\>** when: ( person Paloma AND count=1 ) OR ( person Paloma AND person Emeric AND count>=2 ) MUST EXCLUDE person Manon

The new builder preserves ALL existing predicates (date, location with map widget, people-must-include / may-include / must-exclude, media type, people-count via YOLO, allow / disallow unrecognized faces) and adds the composability the linear form blocks. Old rules continue to load and run unchanged via a `LegacyMatchSpec → MatchExpr` back-compat path at parse time.

Non-negotiable: PRD §6 Appendix A YAMLs must round-trip through the new parser with bit-for-bit identical evaluation behavior. The three deployed rules in production (`3b2b16f1`, `714dce95`, `beba1580`) must keep evaluating to the same decisions after the schema upgrade.

---

## 2. Concrete examples — old YAML vs. new tree YAML

### Example A: PRD Appendix A "Photos of my daughter where only my wife or I are present"

**Old (flat) form** — verbatim from PRD §6 Appendix A:

```yaml
name: "Famille — restreint"
target_album:
  type: managed
  name: "Paloma — Famille proche"
match:
  people:
    must_include: [<paloma-id>]
    may_include: [<manon-id>, <emeric-id>]
    must_exclude_other_identifiable: true
    no_unidentified_humans: true
status: active
```

**New (tree) form** — semantic equivalent:

```yaml
name: "Famille — restreint"
target_album:
  type: managed
  name: "Paloma — Famille proche"
match:
  op: and
  children:
    - { type: person, mode: must_include, person_id: <paloma-id> }
    - { type: face_recognition, allow_unrecognized: false }
status: active
```

**Notes on this translation**:
- `must_include: [<paloma-id>]` → single `person(must_include, paloma-id)` leaf.
- `may_include: [<manon-id>, <emeric-id>]` → captured implicitly by the `face_recognition(allow_unrecognized: false)` semantic combined with the lack of a `must_exclude` block referencing them — i.e., "Paloma must be present; other recognized faces are allowed only if they belong to the recognized-people set; unrecognized humans are rejected". The new schema does NOT need a literal `may_include` block because `face_recognition(allow_unrecognized=false)` already enforces "no faces outside the recognized set", and the recognized set is the union of every `person` block the rule references (any mode) plus whoever Immich has registered globally — see §3 for the precise rule.
- `must_exclude_other_identifiable: true` AND `no_unidentified_humans: true` → both rolled into `face_recognition(allow_unrecognized: false)` (cheap Immich face check + YOLO count check, dispatched in order; see §5).

> **Open Q1**: Should `may_include` survive as a first-class block (parallel to `must_include` / `must_exclude`), or does `face_recognition(allow_unrecognized=false)` adequately express the "only these people allowed" constraint? See §10.

### Example B: PRD Appendix A "Paris trip — week of July 15"

**Old (flat) form**:

```yaml
name: "Paris — juillet 2024"
target_album:
  type: existing
  album_id: <album-uuid>
match:
  date:
    from: 2024-07-15T00:00:00+02:00
    to:   2024-07-22T23:59:59+02:00
  location:
    center: [48.8566, 2.3522]
    radius_km: 60
status: active
```

**New (tree) form**:

```yaml
name: "Paris — juillet 2024"
target_album:
  type: existing
  album_id: <album-uuid>
match:
  op: and
  children:
    - type: date_range
      from: 2024-07-15T00:00:00+02:00
      to:   2024-07-22T23:59:59+02:00
    - type: location
      center: [48.8566, 2.3522]
      radius_km: 60
status: active
```

Straight 1:1 mapping. Implicit AND of two predicates → explicit `op: and` group with two children.

### Example C: PRD Appendix A "All photos of the kids together, no other identifiable humans"

**Old (flat) form**:

```yaml
name: "Enfants ensemble"
target_album:
  type: managed
  name: "Les enfants"
match:
  people:
    must_include: [<kid1-id>, <kid2-id>]
    must_exclude_other_identifiable: true
status: active
```

**New (tree) form**:

```yaml
name: "Enfants ensemble"
target_album:
  type: managed
  name: "Les enfants"
match:
  op: and
  children:
    - { type: person, mode: must_include, person_id: <kid1-id> }
    - { type: person, mode: must_include, person_id: <kid2-id> }
    - { type: face_recognition, allow_unrecognized: false, yolo_count_check: false }
status: active
```

This rule lacks `no_unidentified_humans: true` — only the cheap Immich face check is needed. Hence the explicit `yolo_count_check: false` flag on the face_recognition block (see §3 for the field definition).

### Example D: Operator's directive example — disjunction

> "Include media when: ( person Paloma AND count=1 ) OR ( person Paloma AND person Emeric AND count>=2 ) MUST EXCLUDE person Manon"

```yaml
name: "Paloma seule ou avec Emeric"
target_album:
  type: managed
  name: "Paloma"
match:
  op: and
  children:
    - op: or
      children:
        - op: and
          children:
            - { type: person, mode: must_include, person_id: <paloma-id> }
            - { type: people_count, op: eq, value: 1 }
        - op: and
          children:
            - { type: person, mode: must_include, person_id: <paloma-id> }
            - { type: person, mode: must_include, person_id: <emeric-id> }
            - { type: people_count, op: gte, value: 2 }
    - op: not
      child:
        type: person
        mode: includes
        person_id: <manon-id>
status: active
```

This shape is what the operator wants the UI to read like. The trailing `MUST EXCLUDE` is a top-level NOT branch — see §4 for why excludes are flattened to the top in the UI even when the YAML is nested.

---

## 3. Block taxonomy

Every leaf in the match tree is one of these block types. Each block declares: `type`, its parameters, evaluation cost class, the YAML representation, and the builder UI shape.

| Block | Cost | YAML | UI shape |
|---|---|---|---|
| `person` | cheap (Immich face data) | `{ type: person, mode: <mode>, person_id: <id> }` | Avatar chip + person picker (existing `PeopleMultiSelect` reused single-pick) + mode dropdown |
| `people_count` | YOLO | `{ type: people_count, op: <op>, value: <u32> }` | Op dropdown (== / != / < / <= / > / >=) + number stepper |
| `face_recognition` | cheap (toggle to YOLO via `yolo_count_check`) | `{ type: face_recognition, allow_unrecognized: <bool>, yolo_count_check: <bool> }` | Two toggles in a small card |
| `date_range` | cheap (Immich `takenAt` metadata) | `{ type: date_range, from?: <ISO>, to?: <ISO> }` | Two `<input type="datetime-local">` fields |
| `location` | cheap (Immich exif lat/lng) | `{ type: location, center: [<lat>, <lng>], radius_km: <f64> }` | `<MapPicker>` mounts inline below this block with center marker + radius slider |
| `media_type` | cheap (Immich `type` field) | `{ type: media_type, types: [photo \| video] }` | Two checkboxes (photo, video) |

**Block field semantics** (for the parser/validator in T18):

### `person`
- `mode: must_include` — asset must have this person in its recognized faces. False → tree evaluates `false`.
- `mode: may_include` — asset MAY have this person; never alone determines pass/fail; only meaningful when combined with `face_recognition(allow_unrecognized: false)` where the recognized-set expands to include this person.
- `mode: must_exclude` — asset must NOT have this person.
- `mode: includes` — non-strict include used inside `NOT(...)` constructs ("does this person appear?"). Distinct from `must_include` in that bare `includes(P)` returns the indicator (boolean), not a hard requirement. Only legal as the direct child of a `NOT` node in T18's validator. Outside `NOT`, `includes` is rejected — the operator must use `must_include` instead.
- `person_id: <string>` — the Immich `person.id` UUID. Validator enforces it belongs to the rule owner's Immich account (existing M2 cross-account isolation rule still applies).

### `people_count`
- `op: eq | ne | lt | lte | gt | gte` — comparison operator.
- `value: u32` — non-negative threshold.
- Triggers YOLO inference. Engine dispatch (§5) ensures YOLO is only called when this block needs evaluating after cheaper siblings short-circuit.

### `face_recognition`
- `allow_unrecognized: bool`. When `false`, every face Immich recognized on the asset must belong to the **recognized set** for the rule. The recognized set = the union of all `person_id`s referenced anywhere in the rule's match tree (any mode), plus optionally the `may_include` mode roster. When `true`, this block is a no-op (the default behavior; rule doesn't care if there are unfamiliar faces).
- `yolo_count_check: bool`. Optional, default `false`. When `true` AND `allow_unrecognized=false`, the engine additionally invokes YOLO and rejects the asset when `yolo_count > immich_recognized_count` (the "no unidentified humans" semantic from the old schema). When `false`, only the cheap Immich face data is consulted.
- This block is unique per match tree (validator caps it at one occurrence to keep semantics unambiguous).

### `date_range`
- `from?: <ISO datetime with offset>` — inclusive lower bound (asset's `takenAt` >= from).
- `to?: <ISO datetime with offset>` — inclusive upper bound (asset's `takenAt` <= to).
- At least one of `from`/`to` must be set (validator).

### `location`
- `center: [<lat>, <lng>]` — `[f64; 2]`, lat in `[-90, 90]`, lng in `[-180, 180]` (validator).
- `radius_km: f64` — `> 0` and `<= 20037.5` (half earth circumference; validator).
- Triggers the inline `<MapPicker>` mount in the builder UI when added.

### `media_type`
- `types: [photo | video]` — non-empty (validator). When both are present the block matches any asset; the validator rejects this as a useless block (it's the default if no media_type block is present).

---

## 4. Tree operators

- `op: and` — N children (≥ 2; a single-child AND is rejected by the validator with a helpful "remove the redundant AND wrapper" error). True iff every child evaluates true.
- `op: or` — N children (≥ 2). True iff any child evaluates true. Short-circuits as soon as one returns true.
- `op: not` — exactly one child (`child:`, not `children:`). Inverts the child's boolean. Cannot wrap another `NOT` (validator); the operator can just remove both.

**Nesting depth cap**: 8 levels. The validator counts from the root and rejects deeper trees with `match_tree_too_deep`. Eight is enough for any sane rule but small enough to keep the recursive parser, validator, and evaluator from blowing the stack on adversarial input.

**Leaf vs. group**: every node is either a group (`op:`) or a leaf (`type:`). Mixing both keys at the same level is a parse error (`mixed_node_kind`). A group MUST have a `children:` (or `child:` for `NOT`) array (or single object for `NOT`).

**Empty tree**: a rule with no match section, or a match section with `op: and, children: []`, is rejected by the validator with `empty_match` (carrying forward the PRD §6 rule that empty matches reject everything but cause no useful work).

**Single leaf**: a one-leaf tree is valid and reads in the UI as "Include media when **\<leaf description\>**". E.g., a rule that only filters by date_range parses to `match: { type: date_range, from: ..., to: ... }` — no wrapping `op: and` needed.

**Top-level "must exclude" UI flattening**: when the root is `op: and` and one or more direct children are `op: not`, the UI renders those children as a separate "Must exclude:" section below the main composer, NOT as inline NOT branches. The YAML stays canonical (still `op: and` with NOT children); the UI just splits its rendering. This matches the operator's "MUST EXCLUDE person Manon" trailing-clause intent in Example D.

---

## 5. Engine evaluation strategy

The tree is walked recursively. Each block declares its cost class. The walker dispatches in this order at every `AND` / `OR` node:

1. **Cheap predicates first** (`person`, `face_recognition` without `yolo_count_check`, `date_range`, `location`, `media_type`). Evaluate sequentially; for AND short-circuit on first `false`; for OR short-circuit on first `true`.
2. **Immich-call predicates** — currently none distinct from cheap (Immich face/exif/metadata is already fetched in the asset-batch fetch upstream of evaluation). Reserved for future blocks (e.g., a "smart search" block).
3. **YOLO predicates** (`people_count`, `face_recognition` with `yolo_count_check: true`) — evaluated last. If any cheaper sibling already decided the parent node, YOLO is skipped entirely. **Critical**: an `OR` over (cheap + YOLO) child MUST NOT eagerly fetch YOLO if the cheap child returned `true`. The walker sorts children by cost class per visit, evaluates in cost-ascending order, and only ever invokes the YOLO branch when the answer is still pending.

**YOLO call coalescence**: a single asset may have multiple YOLO-dependent leaves (`people_count(eq, 1)` AND `face_recognition(allow_unrecognized=false, yolo_count_check=true)`). The walker invokes `yolo::infer_person_count` exactly once per asset per cycle, caches the count in the tree evaluator's per-asset scratch state, and re-uses it for all YOLO leaves. The existing `asset_yolo_cache` table (PRD §10) covers cross-cycle persistence; the in-tree cache is just for the duration of evaluating a single asset.

**Decision recording**: the existing `asset_decisions` row layout (PRD §10) gets a richer `reason` taxonomy. Old slug examples: `date_out_of_range`, `location_out_of_radius`, `person_missing`, `person_excluded`, `unidentified_humans_detected`, `media_type_mismatch`. New slugs needed:
- `tree_short_circuit_or` — an OR group rejected because every child returned false. The reason includes the per-child slugs as nested context (JSON-encoded in the same `reason` text column, kept compact: e.g. `{slug: or_fail, children: [{slug: person_missing, person_id: ...}, {slug: people_count_mismatch, op: eq, value: 1, observed: 3}]}`).
- `people_count_mismatch` — `people_count` block disagreed (op, threshold, observed value).
- `not_branch_satisfied` — a NOT child returned true, so the NOT (and its parent) reject.
The legacy slugs continue to be emitted whenever the equivalent leaf rejects. The parser writes both the new tree and the legacy single-cause shape into `reason` so existing dashboards/decision pages keep working without a re-write (forward-compat).

**Watermark behavior**: unchanged. Each rule continues to track its last-processed `takenAt`; the engine_cycle wrapper still pages through Immich, batches assets per cycle, and increments the watermark on completion. The tree evaluator is a drop-in replacement for the current `MatchSpec::evaluate` call (which today is the implicit-AND walk over `date / location / people / media`).

**Per-account isolation**: every `person_id` in any block must belong to the rule owner. Existing M3 cross-account isolation tests apply unchanged — T18's validator extends them to cover trees instead of flat lists.

---

## 6. Back-compat / migration plan

**Two YAML shapes coexist on disk**. Old rules (`match: { date, location, people, media }`) keep parsing. New rules (`match: { op | type, ... }`) parse with the new walker.

Detection in the parser is based on the keys present at the `match:` level:
- If `match` contains `op:` OR `type:` → parse as tree.
- Otherwise → parse as legacy flat spec, then convert to tree via `From<LegacyMatchSpec> for MatchExpr`.

The conversion is deterministic:

| Legacy key | Tree shape |
|---|---|
| (no match) | rejected by validator (`empty_match`) |
| `date: { from, to }` only | `{ type: date_range, from, to }` |
| `location: { center, radius_km }` only | `{ type: location, center, radius_km }` |
| `media: { types }` only | `{ type: media_type, types }` |
| `people.must_include: [A, B]` | AND of `person(must_include, A)`, `person(must_include, B)` |
| `people.must_include_any_of: [A, B]` | OR of `person(must_include, A)`, `person(must_include, B)` — these become `must_include` because the OR semantic carries the "any of" meaning |
| `people.may_include: [A, B]` | inserted as `person(may_include, A)`, `person(may_include, B)` children at the top-level AND, alongside the face_recognition block — purely advisory unless face_recognition(allow_unrecognized=false) is also present |
| `people.must_exclude: [A]` | `op: not, child: { type: person, mode: includes, person_id: A }` |
| `people.must_exclude_other_identifiable: true` | adds `{ type: face_recognition, allow_unrecognized: false, yolo_count_check: false }` |
| `people.no_unidentified_humans: true` | flips the above face_recognition block's `yolo_count_check: true`; if no face_recognition block was inserted yet (i.e., `must_exclude_other_identifiable=false`), inserts `{ type: face_recognition, allow_unrecognized: false, yolo_count_check: true }` |

Multiple legacy keys combine under a top-level `op: and`. Tabular example:

```
Legacy:  match: { date: D, location: L, people: { must_include: [P1], must_exclude: [P2] } }
↓
Tree:
match:
  op: and
  children:
    - { type: date_range, from: D.from, to: D.to }
    - { type: location, center: L.center, radius_km: L.radius_km }
    - { type: person, mode: must_include, person_id: P1 }
    - op: not
      child: { type: person, mode: includes, person_id: P2 }
```

**Conversion is at parse time only**. The legacy YAML stays on disk in the `rules.yaml_source` column until the rule is re-saved through the new builder; at that point, the saved YAML upgrades to the tree shape automatically (the builder always serializes the tree shape, never the legacy shape). This is non-destructive: a rule edited by neither the new builder nor the API stays in legacy YAML, gets re-parsed via the legacy → tree converter each cycle, and evaluates identically.

**Round-trip test matrix** (T18 must include these as integration tests):
1. Each of PRD Appendix A's three YAMLs: parse → tree → re-serialize tree → re-parse → semantic equivalence (same decision on a fixed asset corpus).
2. The two deployed legacy rules `714dce95` and `beba1580`: load their current `yaml_source` → convert to tree → run a synthetic asset corpus → diff decisions against the legacy walker. Must be identical.
3. Adversarial inputs that today's legacy validator catches (`empty_match`, `unknown_field`, etc.) still get rejected by the tree validator with equivalent error messages.

---

## 7. YAML stays source of truth

The DB column `rules.yaml_source` remains the canonical form. The new builder loads it, parses to an `Rc<MatchExpr>` or similar tree IR in-memory, lets the user edit, and re-serializes to YAML on save. The advanced "YAML editor" tab in the builder shows the live YAML (now tree-shaped if the user saved it via the new builder, or legacy-shaped if untouched). Hand-edits through that tab are accepted as long as the parser parses them — including legacy YAML.

The DB does NOT gain a JSON column for the tree. Avoiding schema churn keeps the migration story simple (no per-rule data migration needed) and the parser is the only place that knows about tree-vs-flat duality.

---

## 8. UI sketch

```
┌────────────────────────────────────────────────────────────────────┐
│ Rule: [Famille — restreint                              ]          │
│ Target album:  ( ) Existing  (•) Managed                            │
│                  Album name: [Paloma — Famille proche          ]    │
│ Poll interval: [ 300 ] s  (min 60, max 86400)                       │
├────────────────────────────────────────────────────────────────────┤
│ Include media when:                                                 │
│                                                                     │
│   ┌─ AND ─────────────────────────────────────────────────[+ ↑↓ x]┐│
│   │                                                                ││
│   │  ┌─ person ─────────────────────────────────────────[↑↓ x]──┐ ││
│   │  │ [Paloma ▾] is [must include ▾]                            │ ││
│   │  └──────────────────────────────────────────────────────────┘ ││
│   │                                                                ││
│   │  ┌─ face recognition ──────────────────────────────[↑↓ x]──┐ ││
│   │  │ [✓] All faces must be recognized                          │ ││
│   │  │ [ ] Also reject unrecognized humans (YOLO count check)    │ ││
│   │  └──────────────────────────────────────────────────────────┘ ││
│   │                                                                ││
│   │  [+ Add block ▾]   [+ Add group ▾]                            ││
│   └────────────────────────────────────────────────────────────────┘│
│                                                                     │
│ Must exclude:                                                       │
│   ┌─ person ───────────────────────────────────────────[↑↓ x]──┐  │
│   │ [Manon ▾]                                                    │  │
│   └──────────────────────────────────────────────────────────────┘  │
│   [+ Add exclude]                                                   │
│                                                                     │
├────────────────────────────────────────────────────────────────────┤
│ ▸ Advanced (YAML)                                                   │
│   ┌──────────────────────────────────────────────────────────────┐ │
│   │ name: "Famille — restreint"                                   │ │
│   │ target_album: …                                               │ │
│   │ match:                                                        │ │
│   │   op: and                                                     │ │
│   │   children: …                                                 │ │
│   └──────────────────────────────────────────────────────────────┘ │
│   [Export] [Copy] [Import…]                                         │
├────────────────────────────────────────────────────────────────────┤
│                          [Cancel]  [Save rule]                      │
└────────────────────────────────────────────────────────────────────┘
```

When the user adds a `location` block, a `<MapPicker>` mounts directly under that block's parameters:

```
   │  ┌─ location ───────────────────────────────────[↑↓ x]──┐
   │  │ Center: [48.8566, 2.3522]    Radius: [══●═══] 60 km   │
   │  │ ┌──────────────────────────────────────────────────┐  │
   │  │ │  <MapPicker>  (MapLibre canvas, ~280 px tall)    │  │
   │  │ │  centred + click to set, slider syncs radius     │  │
   │  │ └──────────────────────────────────────────────────┘  │
   │  └──────────────────────────────────────────────────────┘
```

**Controls per node**:
- `[↑↓]` — move within siblings (reorder via arrows; no drag-and-drop in v1).
- `[x]` — remove this block / group.
- On group nodes: `[+ Add block ▾]` picks from the six block types; `[+ Add group ▾]` picks from AND/OR/NOT (NOT is added with a single placeholder child slot to be filled).

**"Add group ▾" menu items**: `AND`, `OR`, `NOT`. Choosing NOT requires picking the child kind (block or group) immediately in a second step.

**Mode switch on a group**: groups have an inline `[AND ▾ | OR | NOT]` button at their top-left corner — clicking flips the operator (NOT only when the group has exactly one child; otherwise greyed out with tooltip "convert this group to AND/OR or remove children").

**Validation feedback**: client-side validation runs on every edit; errors render as red text under the offending block ("Person not in your Immich library", "Empty group not allowed", etc.). On save, the server runs the full validator (T18) and returns structured errors that the UI maps to the same per-block error slots.

**Tree YAML view as truth**: clicking "Advanced (YAML)" expands the textarea showing the canonical tree YAML. Edits in the YAML tab re-parse on blur (with the same validator) and update the visual tree. Round-trip is lossless.

**Loading a legacy rule**: when the page loads a rule whose `yaml_source` is still legacy shape, the builder runs the `LegacyMatchSpec → MatchExpr` conversion in the browser and shows the tree shape. The Advanced YAML tab shows the tree YAML (NOT the original legacy YAML) — saving the rule then writes the tree YAML to the DB, completing the migration. The user does NOT need to manually convert.

---

## 9. Anti-goals (explicitly out of scope for T15..T20)

1. **Drag-and-drop reordering**. Reorder via `[↑↓]` arrow buttons is enough; DnD is implementation-heavy and a known a11y rabbit hole.
2. **Free-text "natural language" rule input** ("include all photos where my daughter is alone"). The English sentence in the operator's example is what the visual builder reads OUT as; it's NOT an input mode.
3. **Block-by-block dry-run preview** (e.g., "this block would match 47 assets"). Deferred to the live activity feed (POSTSHIP-T22/T23) which shows per-rule recent decisions.
4. **Rule templates / cookbook**. No "start from template" picker. The Advanced YAML import button covers the operator's copy-paste workflow.
5. **Versioning / undo within the builder**. No history stack. The rule lifecycle (active/paused/archived) is the only versioning concept.
6. **AI suggestion of blocks** ("we noticed you often add people_count after person blocks — add one?"). PRD §2 non-goals (no third-party AI services).
7. **Schedule editor**. `poll_interval` is a simple number input; cron expressions stay out of scope.

---

## 10. Open questions (operator review required before T18)

These questions don't block T16 (style audit) or T17 (theme primitives) but MUST be answered before T18 (parser) starts. Worker will surface them in JOURNAL and STATE; operator answers them by editing this section or via CLAUDE.md operator directives.

**Q1 — `may_include` as a first-class block?**
Should `may_include` survive as a fourth `person.mode`, or is it adequately expressed by the combination of (no `person(must_include)` block referencing X) AND `face_recognition(allow_unrecognized=false)` (which expands the recognized set to include every person mentioned anywhere)? Worker's recommendation: **keep `may_include` as a fourth mode** because the back-compat conversion needs a 1:1 mapping target and dropping it would force the converter to invent face_recognition blocks that the user didn't intend. Default for T18 unless operator overrides: keep it.

**Q2 — `face_recognition` block: split into two or keep combined?**
Currently spec'd as a single block with `allow_unrecognized: bool` (cheap, Immich face check only) and `yolo_count_check: bool` (adds the YOLO "no extra humans" pass). An alternative is two separate blocks: `face_recognition` (always cheap) and `no_unidentified_humans` (always YOLO). Operator directive §3 lists six block types in T15 spec point 3 — implies the combined form. Default for T18 unless operator overrides: keep combined.

**Q3 — How strict is the `op: or` over (cheap + YOLO) optimization?**
The spec (§5) says YOLO is skipped if a cheaper OR sibling returns true. Is the worker free to also reorder children at evaluation time (cheap sorted before YOLO) so the cheap-wins-fast behavior holds even when the YAML lists YOLO first? Worker's recommendation: **yes, reorder by cost class at every AND/OR visit** (the YAML order is preserved on disk and in serialization, but evaluation order is cost-driven). Tradeoff: a user can no longer write a rule that intentionally forces YOLO to run first for whatever reason (we can't think of a legitimate one). Default for T18 unless operator overrides: reorder by cost class.

**Q4 — Validator depth cap**: 8 levels (§4). Worker proposes 8; operator may want lower (4 or 5) to keep the UI saner. Default: 8.

**Q5 — Reason taxonomy for tree short-circuits**:
The spec (§5) suggests JSON-encoded nested reasons in the `asset_decisions.reason` column ("Decision recording"). Operator may prefer one of: (a) JSON nested (worker recommendation), (b) flat slug only (e.g., `or_fail` with no children), (c) write multiple rows per rejected asset (one per failing branch — explodes the table). Default for T18 unless operator overrides: JSON nested with the legacy slug for forward-compat with existing dashboards.

**Q6 — `RuleStatus` left untouched?**
Active / paused / archived stays exactly as today. No new state. Confirm.

**Q7 — Migration trigger**:
Should re-saving a legacy rule through the new builder be "automatic" (the YAML is now tree-shaped on next GET) or opt-in (rule edits write back in legacy shape unless the user explicitly opts in)? Worker's recommendation: **automatic**. Tradeoff: a rule untouched by the new builder stays legacy on disk forever; one touch and it upgrades. Default: automatic.

---

## 11. Implementation rollout (informational; T18..T20 each ship in one iter)

1. **T18** ships: `crates/engine/src/rule/match_expr.rs` with `MatchExpr` tree type, `From<LegacyMatchSpec>`, full validator, parser tests including PRD Appendix A round-trips, the deployed-rule corpus integration test, and the new reason-slug taxonomy. Old `MatchSpec` continues to compile (used by tests as the legacy input shape). No web changes; existing UI keeps working against legacy YAML (because the API still serves what's in the DB).
2. **T19** ships: the engine cycle's `evaluate_asset` switches from `MatchSpec::evaluate` to `MatchExpr::evaluate`. The legacy walker is dropped (parser converts to tree at load). YOLO coalescence per-asset scratch state added. New decision reasons recorded. Deployed image rebuilt + redeployed; live verify that `714dce95` and `beba1580` still produce the same decision counts post-deploy.
3. **T20** ships: `web/src/pages/rules/RuleBuilder.tsx` rewritten as the tree composer. New components: `BlockTreeEditor.tsx`, `GroupNode.tsx`, `LeafNode.tsx`, six leaf components (`PersonBlock.tsx`, `PeopleCountBlock.tsx`, `FaceRecognitionBlock.tsx`, `DateRangeBlock.tsx`, `LocationBlock.tsx`, `MediaTypeBlock.tsx`). Legacy rules auto-convert in the browser on page mount. Existing `MapPicker.tsx` reused inside `LocationBlock`. Advanced YAML tab unchanged in behavior, gains tree-aware syntax highlighting (just JSON-tree style, not a full linter). Tests: vitest unit tests for the tree serializer in `ruleYaml.test.ts`, and a vitest component test that types into the builder, saves, reloads, and confirms round-trip.
4. **T21** polishes the rest of the app to the Immich theme (T17). The new builder built atop T17's tokens already looks right; T21 brings the rest of the pages in line.

---

## 12. Acceptance for T15 (this doc)

This doc is `[DONE]` when:
- ✅ `docs/design/block-rule-schema.md` exists at the path above.
- ✅ All three PRD Appendix A YAMLs are rewritten in tree form (this file, §2 examples A/B/C).
- ✅ The operator's directive example (Example D) is rewritten in tree form.
- ✅ Each of the six block types is specified (§3).
- ✅ Tree operators with depth cap are specified (§4).
- ✅ Evaluator dispatch order is specified (§5).
- ✅ Back-compat conversion table is specified (§6).
- ✅ ASCII UI wireframe is included (§8).
- ✅ Anti-goals are listed (§9).
- ✅ Open questions for operator are listed (§10).
- Commit `docs(design): block-based rule builder spec` and push.

No code shipped, no Cargo / npm test changes expected. Build/test gates still apply at HEAD: `cargo fmt --all --check`, `cargo clippy --all-targets --workspace -- -D warnings` (no source touched), `cargo test --workspace` (unchanged), `cd web && npm run build/lint/test` (unchanged).

---

*End of design doc.*
