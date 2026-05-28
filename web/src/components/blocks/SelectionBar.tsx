// Floating "Group selected" action bar for the drag-and-drop block builder
// (POSTSHIP-T35, per `docs/design/dnd-block-builder.md` §5.4). This is the
// PRIMARY, deterministic grouping mechanism (drag-to-group is deferred): the
// operator ticks 2+ blocks, then groups them under a new AND/OR sub-group.
//
// The bar owns the determinism guardrails D6 mandates and surfaces them inline:
//   - selection must be SIBLINGS (share a parent) — else AND/OR are disabled
//     with a "same group" hint;
//   - grouping must not bust depth 8 — detected by a dry-run of `wrapInGroup`
//     (which returns the same root ref when it rejects) — else disabled with a
//     depth hint.
// It is presentational over the composer's ephemeral `selection` set: it
// resolves the parent path + child indices from the selected `pathToKey`s and
// emits a fully-resolved `onGroup(parentPath, childIndices, op)`. The composer
// (a later T35 step) applies `wrapInGroup` and clears the selection.

import { Show, createMemo, type Component } from "solid-js";
import type { MatchExpr } from "../../lib/matchTree";
import { keyToPath, parentPath, pathToKey, wrapInGroup } from "../../lib/treeOps";

interface Props {
  root: () => MatchExpr;
  selected: () => ReadonlySet<string>;
  onGroup: (parentPath: number[], childIndices: number[], op: "and" | "or") => void;
  onClear: () => void;
}

const SelectionBar: Component<Props> = (props) => {
  const paths = createMemo(() => [...props.selected()].map(keyToPath));
  const count = () => paths().length;
  const parents = () => paths().map(parentPath);

  // All selected blocks must live in the same parent group to be groupable.
  const sameParent = createMemo(() => {
    const ps = parents();
    if (ps.length < 2) return false;
    const first = pathToKey(ps[0]!);
    return ps.every((p) => pathToKey(p) === first);
  });
  const parent = () => parents()[0] ?? [];
  const childIndices = () => paths().map((p) => p[p.length - 1]!);

  // Depth dry-run: with valid sibling indices the ONLY reason `wrapInGroup`
  // returns the same root is the depth-8 guard, so a same-ref result means the
  // group would nest too deep.
  const depthBlocked = createMemo(() => {
    if (!sameParent()) return false;
    const r = props.root();
    return wrapInGroup(r, parent(), childIndices(), "and") === r;
  });
  const canGroup = () => sameParent() && !depthBlocked();

  const hint = () => {
    if (!sameParent()) return "Select conditions inside the same group to group them.";
    if (depthBlocked()) return "Grouping would exceed the maximum nesting depth.";
    return "";
  };

  const group = (op: "and" | "or") => {
    if (!canGroup()) return;
    props.onGroup(parent(), childIndices(), op);
  };

  return (
    <Show when={count() >= 2}>
      <div
        data-testid="selection-bar"
        role="status"
        aria-live="polite"
        class="sticky bottom-3 z-20 mx-auto flex w-fit max-w-full flex-wrap items-center gap-3 rounded-xl bg-immich-dark-gray px-4 py-2 text-sm text-immich-dark-fg shadow-lg"
      >
        <span class="font-semibold">{count()} selected</span>
        <span class="text-ui-muted">Group as</span>
        <div class="inline-flex overflow-hidden rounded-md border border-white/15">
          <button
            type="button"
            disabled={!canGroup()}
            onClick={() => group("and")}
            aria-label="Group selected as AND"
            title={hint()}
            class="bg-immich-primary px-2.5 py-1 text-xs font-bold uppercase tracking-wide text-white disabled:cursor-not-allowed disabled:opacity-40"
          >
            AND
          </button>
          <button
            type="button"
            disabled={!canGroup()}
            onClick={() => group("or")}
            aria-label="Group selected as OR"
            title={hint()}
            class="bg-amber-500 px-2.5 py-1 text-xs font-bold uppercase tracking-wide text-white disabled:cursor-not-allowed disabled:opacity-40"
          >
            OR
          </button>
        </div>
        <Show when={hint()}>
          <span data-testid="selection-hint" class="text-xs text-amber-300">
            {hint()}
          </span>
        </Show>
        <button
          type="button"
          onClick={() => props.onClear()}
          class="rounded px-2 py-1 text-xs text-immich-dark-fg/80 hover:bg-white/10"
        >
          Clear
        </button>
      </div>
    </Show>
  );
};

export default SelectionBar;
