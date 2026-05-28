# Drag-and-drop block-sentence rule builder — design

**Status**: DESIGN (POSTSHIP-T34). No code shipped. This doc is the contract **POSTSHIP-T35** implements against.
**Governing authority**: `CLAUDE.md` "OPERATOR DIRECTIVES (2026-05-28, POSTSHIP cycle 5) §7" + **LOCKED DECISION D6** (in `.ralph/TASKS.md`). Where this doc conflicts with D6, **D6 wins**. The `MatchExpr` shape is fixed by `docs/design/block-rule-schema.md` (T15) and its TS mirror `web/src/lib/matchTree.ts` (T20) — this builder is a pure view+editor over that IR and **introduces no schema change**.
**Mandatory gate on T35**: LOCKED DECISION **D5** (Chrome-MCP screenshot + critical comparison to this doc's wireframes + the style mirror, *before* the task is marked done — vitest green is not sufficient).

---

## 1. Why this exists (what's wrong with today's builder)

The current composer is `web/src/components/blocks/BlockTreeEditor.tsx` + `GroupNode.tsx` + `BlockShell.tsx` + six leaf blocks. The operator's verdict in cycle-5 directive #7: *"a glorified form, not what was asked."* Concretely, today:

- **Blocks read as forms, not sentences.** A person block is a card titled `PERSON` with a `Mode` `<select>` and a picker stacked vertically (see `PersonBlock.tsx`). It does not read "Paloma is present."
- **No reordering at all.** `AndOrGroupNode` only *appends* (`appendLeaf`/`appendGroup`) and *removes*. There are no `↑↓` arrows and no drag. Order is the insertion order, forever.
- **No way to group existing blocks.** You can add an empty AND/OR group then add children *into* it, but you cannot select two existing siblings and wrap them. Restructuring means deleting and re-adding.
- **No top-level "exclude" lane.** Excludes are just person blocks with `mode: must_exclude` buried inline; there's no blacklist affordance.
- **Dashed-border "draft" aesthetic** (`border-2 border-dashed`) reads as unfinished, not as an Immich management surface.

T35 replaces this with the D6 interaction model: **pill-cards** that read as phrases, **bordered AND/OR/NOT group containers** with visible nesting, **"Group selected" as the primary (deterministic) grouping mechanism**, **drag to reorder**, and a **top-level "Always exclude" strip**.

> **Note on the reversed anti-goal.** `block-rule-schema.md` §9 item 1 listed drag-and-drop as an *anti-goal* for T15–T20 ("DnD is implementation-heavy and a known a11y rabbit hole"). Cycle-5 directive #7 + D6 explicitly reverse that decision. This doc therefore supersedes §9 item 1 of the schema doc. The a11y concern is real and is addressed in §13 (drag is *not* the only path to any operation).

---

## 2. Non-negotiables carried in from the schema doc

These do not change in T35 and the builder must honor them:

- **The IR is `MatchExpr`** (`web/src/lib/matchTree.ts`): `AndGroup`/`OrGroup`/`NotGroup` + six leaves (`person`, `people_count`, `face_recognition`, `date_range`, `location`, `media_type`). Constructors `and`/`or`/`not`, `serializeMatchExpr`, `parseMatchExpr`, `defaultLeaf`/`defaultGroup` (`defaults.ts`) are reused verbatim.
- **YAML stays source of truth.** The builder edits the tree in memory and round-trips through `formStateToYamlV2`/`yamlToFormStateV2` (`web/src/lib/ruleYamlV2.ts`) on every mutation, exactly as `RuleBuilderV2` does today (`syncYaml`). The Advanced (YAML) panel stays two-way bound.
- **Depth cap 8** (`MAX_TREE_DEPTH`). The builder must refuse to create a node deeper than 8 (disable "New sub-group" / reject a drop that would exceed it) and surface a clear message.
- **`AND`/`OR` need ≥ 2 children; `NOT` has exactly one `child`; `includes` mode is NOT-only.** The builder must keep the tree in a server-acceptable shape (or show inline validation when it can't, e.g. an OR with one child).
- **Back-compat**: existing rules (`714dce95`, `beba1580`, the three PRD Appendix A YAMLs, and any legacy-flat rule) load via the existing `parseMatchExpr` → `legacyMatchSpecToTree` path and render in the new builder unchanged. Saving re-serializes the tree shape (the cycle-4 migration behavior — unchanged).

---

## 3. The top-level model: positive expression + "Always exclude" strip

D6 requires the page to read as *"**Include media when** ⟨root group⟩"* with a separate *"**Always exclude**"* strip. We map this onto the existing tree with a **partition of the root AND**, no schema change:

### 3.1 Partition rule (parse / load)

Given the rule's root `MatchExpr`:

- **If the root is an `and` group**: split its direct children into two buckets:
  - **Exclude entries** — any direct child that is shaped as a person blacklist: either a `person` leaf with `mode: must_exclude`, **or** `not(person{mode: includes})`. These render in the **Always exclude** strip.
  - **Positive entries** — everything else. These feed the main composer.
- **If the root is `or`, `not`, or a single leaf**: the exclude strip is empty; the whole root is the positive expression. (Excludes only live as direct children of a top-level AND — matching `block-rule-schema.md` §4 "top-level must-exclude UI flattening" and the deployed rules' actual shape.)

The **positive expression** shown in the main composer is then:
- the single positive entry, if exactly one remains (e.g. Example D: one `or` group remains → the composer's root group *is* that OR group);
- an implicit `and` of the positive entries, if more than one remains (the composer's root group is AND with those children);
- the empty state, if none remain (only excludes were set).

### 3.2 Recombination (serialize / save)

Let `P` = the positive expression the user edited (a single leaf or a group) and `E` = the list of exclude entries from the strip.

- `E` empty → root = `P` (as-is).
- `E` non-empty:
  - `P` is an `and` group → **flatten**: root = `and([...P.children, ...E])`.
  - `P` is an `or` group, `not` group, or a leaf → root = `and([P, ...E])`.

This round-trips the operator's directive example bit-for-bit:

```
Load:  and([ or([...]), not(person includes Manon) ])
       → strip = [Manon];  positive = or([...])  (1 entry, a group → root group is that OR)
Save:  P = or([...]), E = [not(person includes Manon)]
       → root = and([ or([...]), not(person includes Manon) ])   ✓ identical
```

and the deployed `must_exclude`-leaf shape:

```
Load:  and([ <positive...>, person{must_exclude, Manon} ])
       → strip = [Manon];  positive = <positive...>
Save:  → and([ <positive...>, person{must_exclude, Manon} ])     ✓ identical
```

**Canonical shape emitted by the strip for a *new* exclude**: a `person` leaf with `mode: must_exclude` as a direct child of the root AND. Rationale: it is flat, it is exactly what the two deployed rules and `legacyMatchSpecToTree` already produce (`matchTree.ts:470`), and the evaluator treats `person(must_exclude, X)` and `not(person(includes, X))` identically. Both shapes are *recognized* on load; only `person(must_exclude)` is *written*. (Open Q1 — operator may prefer the `not(includes)` shape.)

### 3.3 Why a partition and not a new "exclude" node type

The exclude lane is a *view* over existing tree children, not a new IR concept. This keeps the server schema, validator, and evaluator untouched, keeps YAML canonical, and means a hand-edit in the Advanced panel that adds `person{must_exclude}` at the top level automatically appears in the strip on the next parse. Zero migration.

---

## 4. Drag-and-drop technical approach

### 4.1 Decision: native HTML5 drag events. No new dependency.

The web app is SolidJS 1.9.3 + Vite 6 + Tailwind 3.4, and ships **no DnD library today** (`web/package.json`). We will **not add one**.

| Option | Bundle cost | Verdict |
|---|---|---|
| **Native HTML5 DnD** (`draggable`, `dragstart`/`dragover`/`drop`/`dragend`) | **0 kB** | **Chosen.** Sufficient for a vertical list reorder + drop-into-group. Works with SolidJS event delegation. |
| `@thisbeyond/solid-dnd` | ~12 kB gzip + a reactive store/sensor layer | Rejected. Built for sortable grids/kanban; overkill for a tree of vertical lists, and its sensor model fights our path-addressed mutations. |
| `@neodrag/solid` | ~3 kB | Rejected. Free-position dragging (x/y transforms), not list-insertion semantics — we'd still hand-write the drop math. |

Native DnD gives us, for free: a drag image, the `dragover`/`dragleave` hover lifecycle for drop-zone highlighting, and `dataTransfer` to carry the dragged node's path. The only thing it does *not* give us is touch support (see §13) and polished keyboard semantics (addressed by the "Group selected" + overflow-menu fallbacks, also §13).

### 4.2 Drag mechanics

- **Draggable unit**: each pill-card and each group card. The drag handle (`⠿`, `cursor-grab`) is the visual affordance; the whole card carries `draggable={true}` but `dragstart` is ignored unless it originated on the handle (guard with a `data-drag-handle` check) so that clicking inline `<select>`/`<input>` controls never starts a drag.
- **`dragstart`**: stash the source node's **path** (see §6) in a `dragState` signal *and* `e.dataTransfer.setData("application/x-block-path", pathStr)`. Set `e.dataTransfer.effectAllowed = "move"`. Apply a dim/`opacity-50` class to the source.
- **Drop zones**: two kinds.
  1. **Between-siblings gaps** — a 0-height zone rendered between each pair of siblings (and at the top/bottom of each group body). On `dragover` it thickens into a 2px `bg-immich-primary` **drop line** and `e.preventDefault()` enables the drop. On `drop` → `moveNode(root, from, parentPath, gapIndex)`.
  2. **Group body** — dropping onto a group card's body (not a gap) appends to that group. Highlight the body with `ring-2 ring-immich-primary/40`.
- **`dragend`**: clear `dragState`, remove all highlight/dim classes.
- **Illegal drops are refused** (no `preventDefault`, cursor shows "no-drop"):
  - dropping a node **into its own descendant** (`parentPath` starts with `from`),
  - a drop that would exceed **depth 8**,
  - dropping the `includes`-mode person leaf anywhere outside a NOT (it can only legally live under NOT).
- After any successful drop, the tree is re-normalized (§7) and re-serialized to YAML.

### 4.3 Why drag is *secondary* for grouping

D6 is explicit: *"Drag-to-group is too fiddly as the primary — offer it only as a secondary convenience."* So in v1 (T35), **drag does reordering and moving between groups**. Grouping is done by **selection + "Group selected"** (§5.4), which is deterministic and testable. Drag-onto-a-block-to-wrap-both-in-a-new-group is deferred (Open Q4).

---

## 5. Interaction model (D6, in detail)

### 5.1 Pill-cards (leaf conditions)

Each leaf renders as a single-row **pill-card** that reads as a phrase, with controls inline:

```
[☐] ⠿  👤 [Paloma ▾] is present                                    ✕
[☐] ⠿  🔢 people count [ = ▾] [ 1 ]                                ✕
[☐] ⠿  🙂 all faces recognized  ·  [☐] also reject extra humans    ✕
[☐] ⠿  📅 taken  from [2024-07-15] to [2024-07-22]                 ✕
[☐] ⠿  📍 within [60] km of (48.857, 2.352)            [Map ▾]     ✕
[☐] ⠿  🎞 is a [photo ▾]                                           ✕
```

- `[☐]` selection checkbox (drives "Group selected", §5.4).
- `⠿` drag handle (`cursor-grab`).
- An emoji/icon + the **phrase**, where the variable parts are inline controls (a `<select>` for the person, op, media type; `<input>` for count, dates, radius). The phrase wording is in §11.
- `✕` remove.

The pill is `rounded-xl border border-ui-border bg-white dark:bg-immich-dark-gray px-3 py-2 shadow-sm` — a *filled* surface, not dashed. Selected → `ring-2 ring-immich-primary`. Dragging → `opacity-50`.

**Person phrase wording is mode-driven** (subject reads naturally): `must_include` → "**Paloma** is present", `may_include` → "**Paloma** may be present", `includes` (NOT-only) → "**Paloma** appears", `must_exclude` only ever appears in the exclude strip (§5.5) so its inline wording is the strip's "never **Paloma**". The person name comes from `PeopleContext` (reused); fall back to a short id if the people list hasn't loaded.

### 5.2 Group cards (AND / OR / NOT containers)

```
┌─ AND ▾ ───────────────────────────────  [☐] [NOT ☐]  Remove group ─┐
│  …child blocks, each indented, with AND/OR connectors between them… │
│  + Add condition ▾                                                  │
└─────────────────────────────────────────────────────────────────────┘
```

- Header: an **AND ⇄ OR segmented toggle** (reusing today's two-button toggle from `AndOrGroupNode`: AND = `immich-primary`, OR = `amber-500`), a **NOT** checkbox (wraps/unwraps this group in a `not(...)`), the group's own selection checkbox, and **Remove group**.
- Body: children rendered recursively, separated by small `AND`/`OR` connector chips (kept from today's design — they make the operator visible between rows). Each child is preceded/followed by a between-siblings drop gap (§4.2).
- Footer: **"+ Add condition ▾"** — a single menu (relabel of today's `AddBlockDropdown`) listing: *Person · People count · Unrecognized faces · Date · Location · Media type · ──— · New sub-group (AND) · New sub-group (OR)*. "Unrecognized faces" is the human label for the `face_recognition` leaf.
- **Visual nesting**: each group card gets a **depth-colored left border** (`border-l-4`) plus left padding. Depth palette cycles so siblings-of-siblings stay distinguishable: depth 0 = `immich-primary`, 1 = `ui-info`, 2 = `ui-success`, 3 = `ui-warning`, then repeat. Card chrome is `rounded-2xl border border-ui-border bg-white/60 dark:bg-immich-dark-gray/60`.

**NOT rendering**: `not(child)` renders as a group card with the NOT checkbox checked and a single child slot. Per `block-rule-schema.md` §4, NOT-of-NOT is rejected; the UI greys the NOT checkbox on a group whose parent is already a NOT. NOT children that are `person(includes, X)` at the *top level* are lifted into the exclude strip (§3.1) rather than shown as inline NOT cards.

### 5.3 Adding a condition

"+ Add condition ▾" in any group footer appends a `defaultLeaf(kind)` or `defaultGroup(kind)` (from `defaults.ts`, reused) to that group. The empty-state (no conditions yet) shows one centered "+ Add condition ▾" that seeds the root (matching today's empty-state behavior in `BlockTreeEditor`). Adding a `location` leaf auto-expands its inline map (§5.6).

### 5.4 "Group selected" — the PRIMARY grouping mechanism

This is the deterministic restructuring path D6 mandates:

1. The operator ticks the selection checkbox on **2+ sibling blocks** (pills and/or group cards that share the same parent).
2. A **floating action bar** (sticky to the bottom of the composer, `rounded-xl bg-immich-dark-gray text-immich-dark-fg shadow-lg`) appears: `「 3 selected · Group ▾ (AND | OR) · Clear 」`.
3. Picking AND or OR calls `wrapInGroup(root, parentPath, selectedChildIndices, op)` (§6): the selected children are removed from their parent and replaced, **at the position of the first selected index**, by a new `and`/`or` group containing them in their original relative order. Selection clears.

**Determinism guardrails** (enforced + surfaced in the action bar):
- Selection must be **siblings** (same parent path). If the operator ticks blocks across different parents, "Group ▾" is **disabled** with a tooltip: *"Select conditions inside the same group to group them."* (We compute the common parent; if not all equal, disable.)
- Grouping must not exceed depth 8; if it would, disable with a depth message.
- Grouping a single block is a no-op (the bar only appears at ≥ 2).

Selection is also how you **un-group**: "Remove group" on a group card with the *"keep children"* affordance is out of scope for v1 — to dissolve a group, drag its children out then remove the empty group, or use the Advanced YAML. (Open Q3: add an explicit "Ungroup" that splices children into the parent.)

### 5.5 "Always exclude" strip

A distinct lane below the main composer:

```
Always exclude  (these people are never matched, even if everything else fits)
┌───────────────────────────────────────────────────────────────────────┐
│  🚫 Manon      ✕      🚫 [add a person ▾]                              │
└───────────────────────────────────────────────────────────────────────┘
```

- Renders the exclude entries from the §3.1 partition as removable person chips.
- "+ add a person" appends a `person{mode: must_exclude}` to the root AND (recombination §3.2 makes this a top-level child). If the current root is not yet an AND, recombination wraps it.
- Styled as a blacklist: `rounded-2xl border border-rose-300/60 dark:border-rose-800/60 bg-rose-50/40 dark:bg-rose-900/15`, chips `bg-rose-100 dark:bg-rose-900/40`.
- This lane is **not draggable** and **not selectable** — it's a flat blacklist, deliberately simpler than the composer.

### 5.6 Location block → inline map

When a `location` pill is present, a **"Map ▾"** disclosure on the pill expands the existing `MapPicker` (lazy, reused via today's `LocationBlock` pattern) directly beneath that pill, inside the same group. Collapsing hides the canvas but keeps the center/radius. Adding a location leaf opens it expanded by default (so the operator immediately sees what they're configuring). `MapPicker` already syncs center+radius via `onChange` — no change needed.

---

## 6. Tree operations module (`web/src/lib/treeOps.ts`) — NEW, pure, unit-tested

All structural edits go through a **path-addressed**, pure-function module. This is the heart of T35 and the most testable surface — drag, "Group selected", remove, and reorder all reduce to these.

**Path** = an array of child indices from the root. NOT's single `child` is addressed as index `0`. Examples: `[]` = root, `[2]` = root's 3rd child, `[0,1]` = first child's 2nd child, `[3,0]` = the child of the NOT at root index 3.

```ts
// Read
getNode(root: MatchExpr, path: number[]): MatchExpr | null
parentPath(path: number[]): number[]          // path.slice(0, -1)
isPrefix(a: number[], b: number[]): boolean    // a is an ancestor-or-self of b

// Immutable edits — each returns a new root (structural sharing where cheap)
replaceNode(root, path, next): MatchExpr
removeNode(root, path): MatchExpr              // splices out of children / unwraps NOT
insertChild(root, groupPath, index, node): MatchExpr
moveNode(root, from: number[], toParent: number[], toIndex: number): MatchExpr
wrapInGroup(root, parentPath, childIndices: number[], op: "and" | "or"): MatchExpr
setGroupOp(root, path, op): MatchExpr          // AND<->OR flip
toggleNot(root, path): MatchExpr               // wrap node in NOT / unwrap a NOT
```

**`moveNode` ordering rule** (the subtle one): compute by *removing first, then inserting*, and adjust `toIndex` when the removal shifted indices within the same parent. The implementation removes the source, then if `toParent` equals the old parent and `toIndex` was after the removed index, decrement `toIndex` by 1. Guard: reject (return `root` unchanged) when `isPrefix(from, toParent)` — can't move a node inside itself.

**`wrapInGroup` rule**: sort `childIndices` ascending; collect those children; build `defaultGroup(op)`-shaped node with them; remove them from the parent (highest index first to keep indices valid); insert the new group at the original `min(childIndices)`.

**Normalization** (`normalizeTree(root)`), applied after every edit before serialization:
- An `and`/`or` group with **1 child** → unwrap to the child (avoids the validator's single-child-AND rejection). With **0 children** at the root → the empty state (matches-everything warning); a 0-child non-root group is removed by its parent's edit.
- A `not(emptyMatch)` left dangling (child removed) → drop the NOT.
- Re-derive depth; if an edit produced depth > 8, the edit is rejected upstream (the op returns `root` unchanged and the UI shows a message) — `normalizeTree` never silently truncates.

These functions are framework-agnostic (operate on plain `MatchExpr`), so T35 unit-tests them directly in `treeOps.test.ts` independent of any rendering.

---

## 7. Component architecture — keep vs. rewrite (no orphans)

T35 must leave **no dead files**. Explicit disposition of every current `web/src/components/blocks/*`:

| File | Disposition in T35 |
|---|---|
| `lib/matchTree.ts` | **Keep verbatim.** The IR + parse/serialize/constructors. |
| `lib/ruleYamlV2.ts` | **Keep.** YAML round-trip. |
| `components/MapPicker.tsx` | **Keep.** Reused by the location pill. |
| `components/PeopleContext.tsx` | **Keep.** Person-name lookup for phrases. |
| `blocks/PersonPicker.tsx` | **Keep** (maybe restyle to inline). Used in the person pill + exclude strip. |
| `blocks/defaults.ts` | **Keep + extend.** `defaultLeaf`/`defaultGroup` reused; add a `New sub-group` label and the phrase/icon metadata if not colocated in `phrases.ts`. |
| `blocks/BlockTreeEditor.tsx` | **Rewrite.** Becomes the DnD composer root: renders the positive root group + the Always-exclude strip + the floating selection action bar; owns the `selection` and `dragState` signals and the drag context. Same `{ expr, onChange }` prop contract as today so `RuleBuilderV2` barely changes. |
| `blocks/GroupNode.tsx` (`NodeRenderer`/`AndOrGroupNode`/`NotGroupNode`) | **Rewrite** → `GroupCard.tsx` + `NodeView.tsx`: recursive renderer with drop gaps, depth borders, selection checkboxes, drag handles, the AND/OR toggle + NOT checkbox header. |
| `blocks/BlockShell.tsx` | **Replace** → `PillCard.tsx`: phrase chrome (checkbox + handle + phrase slot + remove), filled (not dashed) surface. |
| `blocks/PersonBlock.tsx`, `PeopleCountBlock.tsx`, `FaceRecognitionBlock.tsx`, `DateRangeBlock.tsx`, `LocationBlock.tsx`, `MediaTypeBlock.tsx` | **Refactor** to render their inline controls *inside* `PillCard` as a phrase (the field logic mostly survives; the chrome + layout change from stacked-form to inline-phrase). |
| `blocks/AddBlockDropdown.tsx` | **Keep + relabel** to "+ Add condition" with the D6 menu items (it already does leaf + group). |

New files T35 adds: `treeOps.ts` (+ test), `phrases.ts` (leaf → phrase string + icon, §11), `PillCard.tsx`, `GroupCard.tsx`/`NodeView.tsx`, `ExcludeStrip.tsx`, `SelectionBar.tsx`, a small `dragContext.ts` (Solid context exposing `dragState` + drop handlers).

`RuleBuilderV2.tsx` host page: **unchanged except internals of the composer** — it still does `<PeopleProvider><BlockTreeEditor expr={expr()} onChange={mutateExpr} /></PeopleProvider>` (line ~499). The exclude strip lives *inside* the rewritten `BlockTreeEditor`, so the host doesn't need a second prop.

---

## 8. State model

The composer owns three reactive pieces (all derived from / writing back to the single `expr` prop):

1. **`expr` (the tree)** — owned by `RuleBuilderV2` (signal), passed in; every edit calls `onChange(normalizeTree(next))` which re-serializes YAML. The composer never holds its own copy of the tree (single source of truth → no desync with the YAML panel).
2. **`selection: Signal<Set<string>>`** — stringified paths (`"0.2"`) of checked blocks. Cleared on any structural edit and on "Clear". Drives the SelectionBar.
3. **`dragState: Signal<{ fromPath: number[] } | null>`** — set on `dragstart`, cleared on `dragend`. Drives drop-gap highlighting and the illegal-drop guards. Exposed via `dragContext` so deeply-nested gaps can read it without prop-drilling.

Selection and drag are **ephemeral UI state**, never serialized.

---

## 9. Wireframes (ASCII, dark mode — light swaps surfaces, keeps structure)

### 9.1 Populated builder (the operator's directive example)

> Rule: *Include when ( Paloma AND count=1 ) OR ( Paloma AND Emeric AND count≥2 ), always exclude Manon.*

```
┌─ AppShell ───────────────────────────────────────────────────────────────────┐
│ Sidebar │  Edit "Paloma seule ou avec Emeric"          [Activity] [Decisions] │
│ Rules•  │  ┌──────────────────────────────────────────────────────────────┐  │
│ Activ.  │  │ Name        [ Paloma seule ou avec Emeric            ]        │  │
│ Settgs  │  │ Target      (•) Managed album   [ Paloma            ]         │  │
│         │  │ Poll        [ 300 ] s                                         │  │
│         │  └──────────────────────────────────────────────────────────────┘  │
│         │                                                                      │
│         │  Include media when                                                 │
│         │  ┃┌─ OR ▾ ───────────────────────────  [☐] [NOT ☐]  Remove group ┐ │
│         │  ┃│ ┃┌─ AND ▾ ───────────────────────  [☐] [NOT ☐]  Remove grp ┐│ │
│         │  ┃│ ┃│  [☐] ⠿ 👤 [Paloma ▾] is present                      ✕ ││ │
│         │  ┃│ ┃│              · AND ·                                     ││ │
│         │  ┃│ ┃│  [☐] ⠿ 🔢 people count [ = ▾] [ 1 ]                  ✕ ││ │
│         │  ┃│ ┃│  + Add condition ▾                                      ││ │
│         │  ┃│ ┃└──────────────────────────────────────────────────────┘│ │
│         │  ┃│              · OR ·                                         │ │
│         │  ┃│ ┃┌─ AND ▾ ───────────────────────  [☐] [NOT ☐]  Remove grp ┐│ │
│         │  ┃│ ┃│  [☐] ⠿ 👤 [Paloma ▾] is present                      ✕ ││ │
│         │  ┃│ ┃│  [☐] ⠿ 👤 [Emeric ▾] is present                      ✕ ││ │
│         │  ┃│ ┃│  [☐] ⠿ 🔢 people count [ ≥ ▾] [ 2 ]                  ✕ ││ │
│         │  ┃│ ┃│  + Add condition ▾                                      ││ │
│         │  ┃│ ┃└──────────────────────────────────────────────────────┘│ │
│         │  ┃│ + Add condition ▾                                          │ │
│         │  ┃└────────────────────────────────────────────────────────────┘ │
│         │                                                                      │
│         │  Always exclude  (never matched, even if everything else fits)      │
│         │  ┌──────────────────────────────────────────────────────────────┐  │
│         │  │ 🚫 Manon  ✕      🚫 + add a person ▾                          │  │
│         │  └──────────────────────────────────────────────────────────────┘  │
│         │                                                                      │
│         │  ▸ Advanced (YAML)                                                  │
│         │                                  [ Cancel ]  [ Save rule ]          │
└─────────┴──────────────────────────────────────────────────────────────────────┘
  ┃ = depth-colored left border (primary → info → success → warning, cycling)
```

### 9.2 Empty state

```
│  Include media when                                                 │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │   No conditions yet — this rule would match every asset.       │ │
│  │                    [ + Add condition ▾ ]                       │ │
│  └────────────────────────────────────────────────────────────────┘ │
│  Always exclude   🚫 + add a person ▾                               │
```

### 9.3 "Group selected" floating action bar (2 pills ticked)

```
│  ┃┌─ AND ▾ ──────────────────────────────────  [☐] [NOT ☐]  Remove ┐│
│  ┃│  [☑] ⠿ 👤 Paloma is present                                  ✕ ││  ← ticked (ring)
│  ┃│  [☑] ⠿ 🔢 people count = 1                                   ✕ ││  ← ticked (ring)
│  ┃│  [☐] ⠿ 📅 taken in 2024                                      ✕ ││
│  ┃└────────────────────────────────────────────────────────────────┘│
│                                                                       │
│       ╭──────────────────────────────────────────────────────────╮  │ ← sticky action bar
│       │  2 selected     Group ▾  ( AND | OR )       Clear          │  │
│       ╰──────────────────────────────────────────────────────────╯  │
```
→ picking **AND** wraps the two ticked pills into a new AND sub-group at the first one's position; "taken in 2024" stays where it was.

### 9.4 Drag in progress (reorder + drop line)

```
│  ┃│  [☐] ⠿ 👤 Paloma is present                                  ✕ ││  (opacity-50, lifted)
│  ┃│  ══════════════════════════════════════════════════════════  ││  ← 2px primary drop line
│  ┃│  [☐] ⠿ 🔢 people count = 1                                   ✕ ││
```

### 9.5 Location pill expanded

```
│  ┃│  [☐] ⠿ 📍 within [ 60 ] km of (48.857, 2.352)      [Map ▴]   ✕ ││
│  ┃│  ┌──────────────────────────────────────────────────────────┐ ││
│  ┃│  │  <MapPicker>  (MapLibre, ~280px; click=center, slider=km) │ ││
│  ┃│  └──────────────────────────────────────────────────────────┘ ││
```

---

## 10. Visual design (tokens from `immich-style-mirror.md`)

- **Surfaces**: body `bg-immich-bg` / `dark:bg-immich-dark-bg` (#0a0a0a). Pills + group cards `bg-white` / `dark:bg-immich-dark-gray` (#212121). Dark mode separates by *surface tone*, so group cards use a slightly translucent fill (`/60`) over the body to read as nested.
- **Radii**: pills `rounded-xl` (12px — Immich's signature input radius), group cards `rounded-2xl` (16px), the SelectionBar `rounded-xl`.
- **Primary accent**: `immich-primary` (#4250af light) / `immich-dark-primary` (#accbfa dark) — drop line, AND toggle, selected ring, "Save rule".
- **OR accent**: `amber-500` (kept from today's `AndOrGroupNode`).
- **Depth borders**: `border-l-4` cycling `immich-primary → ui-info → ui-success → ui-warning`.
- **Exclude lane**: rose family (`rose-300`/`rose-900`) to read as a blacklist, consistent with today's `NotGroupNode` tinting.
- **Type**: Overpass (inherited). Phrases at `text-sm`; the operator chip (AND/OR) at `text-[11px] font-bold uppercase tracking-wide`.
- **Drag handle / muted controls**: `text-ui-muted`. Remove `✕`: `text-ui-danger` on hover.
- **Motion**: `transition` at the 0.15s Immich default; drop line + ring appear instantly (no transition) so the operator gets crisp feedback.

The intent: it should look like an **Immich management surface** (filled cards, near-black dark, blue accent, generous radius), not a wireframe of dashed boxes.

---

## 11. Phrase rendering (`phrases.ts`)

`leafPhrase(leaf, peopleLookup): { icon: string; parts: PhrasePart[] }` where parts are either static text or an inline control descriptor. Wording table:

| Leaf | Icon | Phrase (controls in 〔 〕) |
|---|---|---|
| `person` must_include | 👤 | 〔person ▾〕 **is present** |
| `person` may_include | 👤 | 〔person ▾〕 **may be present** |
| `person` includes (NOT-only) | 👤 | 〔person ▾〕 **appears** |
| `people_count` | 🔢 | **people count** 〔op ▾〕 〔value〕 |
| `face_recognition` allow_unrecognized=false | 🙂 | **all faces must be recognized** · 〔☐ also reject extra humans (YOLO)〕 when yolo_count_check |
| `face_recognition` allow_unrecognized=true, yolo_count_check=true | 🙂 | **no unidentified extra humans** (YOLO) |
| `date_range` | 📅 | **taken** 〔from〕 〔to〕 (omit an absent bound: "taken after 2024-07-15") |
| `location` | 📍 | **within** 〔km〕 **km of** (lat, lng) 〔Map ▾〕 |
| `media_type` | 🎞 | **is a** 〔photo / video ▾〕 (both → "is a photo or video") |

`op ▾` maps `eq→"="`, `ne→"≠"`, `lt→"<"`, `lte→"≤"`, `gt→">"`, `gte→"≥"`. Exclude-strip chips read **"never 〔person〕"** (the `must_exclude` case never renders as an inline composer pill).

---

## 12. Migration / back-compat

No data migration. Existing rules load through `parseMatchExpr` (which already routes legacy-flat YAML through `legacyMatchSpecToTree`), then the §3.1 partition splits the tree for display. Verified mentally against the deployed rules and Appendix A:

- **`714dce95` / `beba1580`** (person + face-recognition style rules): load as a positive AND of person/face pills; any `must_exclude` person surfaces in the strip. Save re-emits the same tree → no behavior change (the cycle-4/cycle-5 evaluator path is untouched).
- **Appendix A "Paris trip"** (date + location): loads as an AND root group with a date pill + a location pill (map collapsed); strip empty.
- **Appendix A "Famille — restreint"** and **"Enfants ensemble"**: load as AND of person pills + a face-recognition pill.
- **Directive Example D**: round-trips per §3.2 (verified above).

The **Advanced (YAML) panel stays two-way bound** exactly as today: hand-edits re-parse and repaint the blocks (including the strip); block edits re-serialize. A YAML the visual builder can't fully represent still round-trips losslessly through the panel (the panel is the escape hatch). T35 keeps `RuleBuilderV2`'s existing `untouched`-keys amber notice for any preserved-but-not-shown YAML.

---

## 13. Accessibility & input-modality (the a11y rabbit hole, addressed)

Native HTML5 DnD is mouse-only and screen-reader-hostile. We mitigate by **never making drag the only path to an operation**:

- **Reorder** without drag: each pill/group's overflow has **"Move up" / "Move down"** menu items (keyboard-reachable, call `moveNode` within the parent). Drag is the fast path; these are the accessible path.
- **Group** without drag: "Group selected" is checkbox + button — fully keyboard-operable and the *primary* mechanism anyway (§5.4).
- **Move across groups** without drag: deferred secondary "Move to group ▾" menu item if the operator asks; v1 ships move-up/down within a group + group/ungroup, which covers the common restructure. (Open Q3/Q5.)
- **Touch**: native DnD doesn't fire on touch. The checkbox-grouping + overflow-move paths work on touch, so the builder is *usable* on a tablet even though drag isn't. Flagged as Open Q2 — if the operator needs polished touch drag we revisit a pointer-event lib.
- Drag handles get `aria-label`, checkboxes get labels naming the block ("select Paloma is present"), the AND/OR toggle uses `aria-pressed` (kept from today), the SelectionBar is an `aria-live` region announcing "2 selected".

---

## 14. Test plan for T35 (vitest)

Pure-function core (no DOM):
- `treeOps.test.ts` — `getNode`/`replaceNode`/`removeNode`/`insertChild`; `moveNode` including the same-parent index-shift rule and the move-into-descendant rejection; `wrapInGroup` for non-contiguous selection; `toggleNot` wrap/unwrap; `normalizeTree` single-child unwrap + dangling-NOT cleanup + depth-8 rejection.
- `phrases.test.ts` — each leaf → expected phrase; op symbols; both-media-types wording; absent date bound.
- Partition round-trip in `ruleYamlV2`/composer: Example D, the two deployed-rule YAMLs, and all three Appendix A YAMLs → load → partition → recombine → **deep-equal the original parsed tree** (the back-compat guarantee).

Component (`@solidjs/testing-library`):
- Add a condition appends the right `defaultLeaf`; "New sub-group" adds an empty group.
- Tick 2 sibling pills → SelectionBar shows "2 selected"; Group→AND wraps them; tree reflects it; selection clears.
- Tick blocks across different parents → "Group ▾" disabled.
- Reorder via "Move up"/"Move down" mutates order; drag (simulate `dragstart`/`dragover`/`drop` with a stubbed `dataTransfer`) reorders and rejects into-descendant drops.
- "+ add a person" in the strip writes a top-level `person{must_exclude}`; removing the chip removes it; loading a rule with a top-level exclude shows the chip and keeps it out of the composer body.
- Location pill expand mounts `MapPicker`; `onChange` updates center/radius and the YAML.
- Load a legacy-flat rule → renders pills (not a YAML dump); edit + save serializes the tree shape.
- AND↔OR toggle + NOT checkbox mutate the group op / wrapping.

Gates (must stay green): `npm run typecheck`, `npm run lint` (0 warnings), `npm test -- --run`, `npm run build` (note bundle delta — expect a small increase from the new components, *no* DnD-library weight). Workspace Rust gates unaffected (web-only change).

## 15. D5 self-check plan (MANDATORY before T35 is marked done)

Per LOCKED DECISION D5, vitest-green is necessary but not sufficient. Before committing T35:
1. Build the SPA, render the real `RuleBuilderV2` in the real `AppShell` (reuse the throwaway `web/devpreview/` Vite + Python-Playwright recipe from T30–T33 — `MemoryRouter` seeded at `/rules/:id` with a stubbed rule = the Example-D tree + a stubbed people list so phrases resolve).
2. Screenshot **dark + light**: (a) the populated builder matching §9.1, (b) the "Group selected" action bar, (c) a drag mid-flight with the drop line, (d) the location pill expanded with the map, (e) the exclude strip with a chip.
3. **Critically compare** to §9 wireframes + `immich-style-mirror.md`: filled cards (not dashed), `#0a0a0a`/`#212121` dark surfaces, `#4250af`/`#accbfa` accent, `rounded-xl`/`2xl`, depth borders legible, phrases reading as English (not form labels). Ask: *"Would a non-technical operator look at this and understand the sentence? Does it look like Immich or a generic form?"* Iterate within T35 if it reads form-y.
4. Save screenshots to `docs/postship/` + a verify doc; cite them when closing T35.
5. Kill the dev server by its listener PID (`ss -ltnp | grep :5174`), **not** `pkill -f` (the T31 self-match gotcha).

---

## 16. Open questions for the operator

1. **Exclude canonical shape** — the strip *writes* `person{mode: must_exclude}` (flat, matches deployed rules). The schema doc §6 design intent was `not(person{includes})`. Both evaluate identically and both are *read*. Keep writing the flat `must_exclude` shape? (Worker recommendation: **yes** — consistency with the running rules and `legacyMatchSpecToTree`.)
2. **Touch drag** — v1 ships mouse drag + keyboard/checkbox fallbacks; touch can group + move-up/down but not drag. Acceptable, or is polished touch drag required (would justify a small pointer-event lib)? (Recommendation: **acceptable** — this is a desktop admin tool.)
3. **Explicit "Ungroup"** — should a group card offer "Ungroup" that splices its children into the parent (vs. today's drag-out-then-remove)? (Recommendation: **add it** — cheap via `treeOps`, big usability win. Could fold into T35 or defer.)
4. **Drag-to-group** as a secondary convenience (drop one pill *onto* another → wrap both in a new group)? D6 calls it "fiddly" and makes selection primary; v1 omits it. Add later? (Recommendation: **defer**.)
5. **Cross-group move via menu** — besides drag, offer "Move to group ▾"? (Recommendation: **defer** unless requested; drag covers it for mouse users.)
6. **`may_include` pill** — keep "may be present" as a visible, addable mode in the composer, or hide it (it's only meaningful with face-recognition and confuses non-technical users)? (Recommendation: **keep addable** but low in the menu, since back-compat needs the 1:1 mapping — already settled by schema Open Q1.)

---

## 17. Acceptance for T34 (this doc)

`[DONE]` when:
- ✅ `docs/design/dnd-block-builder.md` exists at this path.
- ✅ DnD approach chosen + justified (native HTML5, no new dep) — §4.
- ✅ Pill-card + group-container + "Group selected" + drag-reorder + Always-exclude strip + inline location map all specified per D6 — §5.
- ✅ Mapping to the existing `MatchExpr` IR with no schema change, incl. the top-level partition + round-trip — §3, §12.
- ✅ The pure `treeOps` mutation surface specified — §6.
- ✅ Keep-vs-rewrite for every current `blocks/*` file (no orphans) — §7.
- ✅ ASCII wireframes (dark mode, style-mirror convention) — §9.
- ✅ Migration / back-compat for the deployed + Appendix A rules — §12.
- ✅ Open questions for the operator — §16.
- Commit `docs(design): drag-and-drop block-sentence builder` and push.

No code shipped. Build/test gates at HEAD unchanged (no source touched): `cargo fmt --all --check`, `cargo clippy --all-targets --workspace -- -D warnings`, `cargo test --workspace`, `cd web && npm run build/lint/test`.

---

*End of design doc. Authored 2026-05-28 by POSTSHIP-T34. Implemented by T35.*
