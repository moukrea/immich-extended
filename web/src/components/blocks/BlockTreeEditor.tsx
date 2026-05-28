// Composer root for the drag-and-drop sentence block rule builder
// (POSTSHIP-T35, per `docs/design/dnd-block-builder.md` §3 / §5 / §8). Same
// `{ expr, onChange }` prop contract as the old flat builder so `RuleBuilderV2`
// is unchanged — the redesign lives entirely below this boundary.
//
// It owns the two ephemeral edit signals (`selection`, `dragFrom`), builds the
// `TreeEditCtx` the recursive `NodeView` reads/writes, and performs the §3
// top-level partition: a root AND is split into a POSITIVE expression (the main
// composer, "Include media when …") and EXCLUDE entries (the rose "Always
// exclude" strip). Positive edits recombine with the current excludes before
// reaching `onChange`; the excludes never appear as inline pills.

import { Show, createMemo, createSignal, type Component } from "solid-js";
import { and, emptyMatch, type MatchExpr } from "../../lib/matchTree";
import { pathToKey, wrapInGroup } from "../../lib/treeOps";
import AddBlockDropdown from "./AddBlockDropdown";
import ExcludeStrip, { type ExcludeEntry } from "./ExcludeStrip";
import NodeView, { type TreeEditCtx } from "./NodeView";
import SelectionBar from "./SelectionBar";
import { defaultLeaf, type AddableLeafKind } from "./defaults";

interface Props {
  expr: MatchExpr;
  onChange: (next: MatchExpr) => void;
}

export interface Partition {
  /** The expression rendered in the main composer (NodeView at path `[]`). */
  positive: MatchExpr;
  /** The original exclude child nodes, in order — reused verbatim on recombine. */
  excludeNodes: MatchExpr[];
  /** Strip view-models; `key` is the exclude child's index in `excludeNodes`. */
  excludes: ExcludeEntry[];
}

// The two top-level "blacklist" shapes the strip owns: a flat
// `person{must_exclude}` leaf (what `legacyMatchSpecToTree` and the deployed
// rules emit) or `not(person{includes})` (the schema-doc design intent). Both
// evaluate identically and both are lifted into the Always-exclude strip; only
// the flat shape is written for a *new* exclude (§3.2 / Open Q1).
function excludePersonId(node: MatchExpr): string | null {
  if (node.kind === "leaf" && node.leaf === "person" && node.mode === "must_exclude") {
    return node.person_id;
  }
  if (node.kind === "group" && node.op === "not") {
    const c = node.child;
    if (c.kind === "leaf" && c.leaf === "person" && c.mode === "includes") {
      return c.person_id;
    }
  }
  return null;
}

// §3.1 — partition the root AND into positive + exclude buckets. A non-AND root
// (or / not / leaf) is wholly positive with no excludes. Exported for the §14
// load→partition→recombine round-trip test.
export function partitionRoot(root: MatchExpr): Partition {
  if (root.kind === "group" && root.op === "and") {
    const positiveChildren: MatchExpr[] = [];
    const excludeNodes: MatchExpr[] = [];
    const excludes: ExcludeEntry[] = [];
    root.children.forEach((child) => {
      const pid = excludePersonId(child);
      if (pid !== null) {
        excludes.push({ key: pathToKey([excludeNodes.length]), person_id: pid });
        excludeNodes.push(child);
      } else {
        positiveChildren.push(child);
      }
    });
    let positive: MatchExpr;
    if (positiveChildren.length === 0) positive = emptyMatch();
    else if (positiveChildren.length === 1) positive = positiveChildren[0]!;
    else positive = and(positiveChildren);
    return { positive, excludeNodes, excludes };
  }
  return { positive: root, excludeNodes: [], excludes: [] };
}

// §3.2 — fold the (possibly edited) positive expression back together with the
// excludes. A top-level positive AND is flattened so excludes stay direct
// children of the root AND (matching the deployed shape + round-tripping bit-
// for-bit); anything else is wrapped. Exported for the §14 round-trip test.
export function recombine(positive: MatchExpr, excludeNodes: MatchExpr[]): MatchExpr {
  if (excludeNodes.length === 0) return positive;
  if (positive.kind === "group" && positive.op === "and") {
    return and([...positive.children, ...excludeNodes]);
  }
  return and([positive, ...excludeNodes]);
}

// A childless AND/OR positive is the "no conditions yet" empty state (§9.2).
function isEmptyPositive(positive: MatchExpr): boolean {
  return (
    positive.kind === "group" && positive.op !== "not" && positive.children.length === 0
  );
}

const BlockTreeEditor: Component<Props> = (props) => {
  const [selection, setSelection] = createSignal<Set<string>>(new Set());
  const [dragFrom, setDragFrom] = createSignal<number[] | null>(null);

  const part = createMemo(() => partitionRoot(props.expr));
  const positive = () => part().positive;

  const clearSelection = () => setSelection(new Set<string>());

  // Every positive-side edit recombines with the current excludes and clears
  // the (now possibly stale-pathed) selection — §8.
  const applyPositive = (nextPositive: MatchExpr) => {
    if (nextPositive === positive()) return;
    props.onChange(recombine(nextPositive, part().excludeNodes));
    clearSelection();
  };

  const ctx: TreeEditCtx = {
    root: positive,
    onChange: (next) => applyPositive(next),
    isSelected: (k) => selection().has(k),
    setSelected: (k, on) =>
      setSelection((prev) => {
        const s = new Set(prev);
        if (on) s.add(k);
        else s.delete(k);
        return s;
      }),
    dragFrom,
    setDragFrom: (p) => setDragFrom(() => p),
  };

  // Top-level "+ Add condition" — leaves only (groups emerge from "Group
  // selected", per D6). Seeds the empty state, or wraps a single positive leaf
  // into an AND so the next addition becomes a sibling.
  const addFirstLeaf = (kind: AddableLeafKind) => applyPositive(defaultLeaf(kind));
  const wrapAndAppendLeaf = (kind: AddableLeafKind) =>
    applyPositive(and([positive(), defaultLeaf(kind)]));

  const onGroup = (parentPath: number[], childIndices: number[], op: "and" | "or") =>
    applyPositive(wrapInGroup(positive(), parentPath, childIndices, op));

  const addExclude = (personId: string) => {
    if (!personId) return;
    const node: MatchExpr = {
      kind: "leaf",
      leaf: "person",
      mode: "must_exclude",
      person_id: personId,
    };
    props.onChange(recombine(positive(), [...part().excludeNodes, node]));
    clearSelection();
  };
  const removeExclude = (key: string) => {
    const idx = part().excludes.findIndex((e) => e.key === key);
    if (idx < 0) return;
    const remaining = part().excludeNodes.filter((_, i) => i !== idx);
    props.onChange(recombine(positive(), remaining));
    clearSelection();
  };

  return (
    <div data-testid="block-tree-editor" class="space-y-4">
      <div class="space-y-2">
        <Show
          when={!isEmptyPositive(positive())}
          fallback={
            <div class="rounded-2xl border-2 border-dashed border-ui-border bg-slate-50/50 p-6 text-center dark:bg-gray-900/40">
              <p class="mb-3 text-sm text-ui-muted dark:text-gray-400">
                No conditions yet — this rule would match every asset.
              </p>
              <AddBlockDropdown
                label="+ Add condition"
                groupKinds={[]}
                onAddLeaf={addFirstLeaf}
                onAddGroup={() => undefined}
              />
            </div>
          }
        >
          <NodeView ctx={ctx} path={[]} />
          <Show when={positive().kind === "leaf"}>
            <div>
              <AddBlockDropdown
                label="+ Add condition"
                groupKinds={[]}
                onAddLeaf={wrapAndAppendLeaf}
                onAddGroup={() => undefined}
              />
            </div>
          </Show>
        </Show>
      </div>

      <SelectionBar
        root={positive}
        selected={selection}
        onGroup={onGroup}
        onClear={clearSelection}
      />

      <ExcludeStrip
        entries={() => part().excludes}
        onAddPerson={addExclude}
        onRemove={removeExclude}
      />
    </div>
  );
};

export default BlockTreeEditor;
