// AND / OR group container for the drag-and-drop block builder (POSTSHIP-T35,
// per `docs/design/dnd-block-builder.md` §5.2). Renders one AND/OR group as a
// bordered, depth-colored card: a header (drag handle + AND/OR segmented
// toggle + NOT checkbox + select + Remove group), a recursive body of children
// separated by AND/OR connector chips with between-sibling drop gaps, and a
// "+ Add condition" footer.
//
// A NOT wrapping this group is rendered IN this card: `notPath` is the path of
// the enclosing `not(...)` node (the NOT checkbox is then checked, and the
// card's identity/selection/removal address that outer NOT). All edits go
// through the path-addressed `treeOps`.

import { Index, Show, createSignal, type Component } from "solid-js";
import type { AndGroup, MatchExpr, OrGroup } from "../../lib/matchTree";
import {
  getNode,
  insertChild,
  isPrefix,
  moveNode,
  parentPath,
  removeNode,
  setGroupOp,
  toggleNot,
} from "../../lib/treeOps";
import AddBlockDropdown from "./AddBlockDropdown";
import { defaultGroup, defaultLeaf, type AddableGroupKind, type AddableLeafKind } from "./defaults";
import NodeView, { type TreeEditCtx } from "./NodeView";

// Depth-colored left border, cycling so siblings-of-siblings stay distinct.
const DEPTH_BORDER = [
  "border-immich-primary",
  "border-ui-info",
  "border-ui-success",
  "border-ui-warning",
];

interface Props {
  ctx: TreeEditCtx;
  groupPath: number[];
  notPath: number[] | null;
  selected: boolean;
  onSelectedChange: (on: boolean) => void;
}

const ConnectorChip: Component<{ op: "and" | "or" }> = (props) => (
  <div class="py-0.5 text-center">
    <span
      class={`rounded-full px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide ${
        props.op === "and"
          ? "bg-immich-primary/10 text-immich-primary dark:bg-immich-dark-primary/20 dark:text-immich-dark-primary"
          : "bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-200"
      }`}
    >
      {props.op}
    </span>
  </div>
);

// A 0-height drop target between two siblings (and at the top/bottom of a
// group body). Thickens into a 2px primary line while a legal drag hovers it.
const DropGap: Component<{ ctx: TreeEditCtx; groupPath: number[]; index: number }> = (props) => {
  const [over, setOver] = createSignal(false);
  const canDrop = () => {
    const f = props.ctx.dragFrom();
    return !!f && !isPrefix(f, props.groupPath);
  };
  return (
    <div
      aria-hidden="true"
      data-testid={`drop-gap-${props.groupPath.join("_")}-${props.index}`}
      class="relative h-2 -my-1"
      onDragOver={(e) => {
        if (!canDrop()) return;
        e.preventDefault();
        e.stopPropagation();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        setOver(false);
        const f = props.ctx.dragFrom();
        if (f) {
          const next = moveNode(props.ctx.root(), f, props.groupPath, props.index);
          if (next !== props.ctx.root()) props.ctx.onChange(next);
        }
        props.ctx.setDragFrom(null);
      }}
    >
      <Show when={over()}>
        <div class="absolute inset-x-0 top-1/2 h-0.5 -translate-y-1/2 rounded bg-immich-primary" />
      </Show>
    </div>
  );
};

const GroupCard: Component<Props> = (props) => {
  const [bodyOver, setBodyOver] = createSignal(false);

  const group = () => getNode(props.ctx.root(), props.groupPath) as AndGroup | OrGroup;
  const op = () => group().op;
  const children = () => group().children;
  const outerPath = () => props.notPath ?? props.groupPath;
  const depthClass = () => DEPTH_BORDER[outerPath().length % DEPTH_BORDER.length]!;
  const notChecked = () => props.notPath !== null;

  const apply = (next: MatchExpr) => {
    if (next !== props.ctx.root()) props.ctx.onChange(next);
  };

  const switchOp = (next: "and" | "or") => {
    if (op() === next) return;
    apply(setGroupOp(props.ctx.root(), props.groupPath, next));
  };

  // NOT-of-NOT is illegal: grey the box when this (un-negated) group already
  // sits directly inside a NOT.
  const notDisabled = () => {
    if (notChecked()) return false;
    const par = getNode(props.ctx.root(), parentPath(props.groupPath));
    return !!par && par.kind === "group" && par.op === "not";
  };
  const toggleNotBox = () => {
    if (notChecked()) apply(toggleNot(props.ctx.root(), props.notPath!));
    else apply(toggleNot(props.ctx.root(), props.groupPath));
  };

  const removeGroup = () => apply(removeNode(props.ctx.root(), outerPath()));

  const addLeaf = (kind: AddableLeafKind) =>
    apply(insertChild(props.ctx.root(), props.groupPath, children().length, defaultLeaf(kind)));
  const addGroup = (kind: AddableGroupKind) =>
    apply(insertChild(props.ctx.root(), props.groupPath, children().length, defaultGroup(kind)));

  const canDropBody = () => {
    const f = props.ctx.dragFrom();
    return !!f && !isPrefix(f, props.groupPath);
  };

  return (
    <div
      data-testid={`groupcard-${op()}`}
      class={`rounded-2xl border border-l-4 border-ui-border ${depthClass()} bg-white/60 p-3 dark:bg-immich-dark-gray/60 ${
        notChecked() ? "ring-1 ring-rose-300 dark:ring-rose-800" : ""
      }`}
    >
      <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center gap-2">
          <span
            data-drag-handle
            aria-label="Drag group to reorder"
            title="Drag to reorder"
            class="cursor-grab select-none text-ui-muted"
          >
            ⠿
          </span>
          <div class="inline-flex overflow-hidden rounded-md border border-ui-border text-xs">
            <button
              type="button"
              aria-label="Use AND"
              aria-pressed={op() === "and"}
              onClick={() => switchOp("and")}
              class={
                op() === "and"
                  ? "bg-immich-primary px-2 py-0.5 text-white"
                  : "bg-white px-2 py-0.5 text-immich-fg dark:bg-gray-700 dark:text-immich-dark-fg"
              }
            >
              AND
            </button>
            <button
              type="button"
              aria-label="Use OR"
              aria-pressed={op() === "or"}
              onClick={() => switchOp("or")}
              class={
                op() === "or"
                  ? "bg-amber-500 px-2 py-0.5 text-white"
                  : "bg-white px-2 py-0.5 text-immich-fg dark:bg-gray-700 dark:text-immich-dark-fg"
              }
            >
              OR
            </button>
          </div>
        </div>
        <div class="flex items-center gap-3 text-xs">
          <label class="inline-flex items-center gap-1 text-ui-muted">
            <input
              type="checkbox"
              checked={props.selected}
              aria-label="Select group"
              onChange={(e) => props.onSelectedChange(e.currentTarget.checked)}
            />
            select
          </label>
          <label
            class={`inline-flex items-center gap-1 ${
              notDisabled() ? "text-ui-muted/50" : "text-ui-muted"
            }`}
          >
            <input
              type="checkbox"
              checked={notChecked()}
              disabled={notDisabled()}
              aria-label="Negate group (NOT)"
              onChange={toggleNotBox}
            />
            NOT
          </label>
          <button
            type="button"
            onClick={removeGroup}
            aria-label="Remove group"
            class="rounded text-ui-danger hover:underline"
          >
            Remove group
          </button>
        </div>
      </div>

      <div
        class={`pl-3 ${bodyOver() ? "rounded-lg ring-2 ring-immich-primary/40" : ""}`}
        onDragOver={(e) => {
          if (!canDropBody()) return;
          e.preventDefault();
          e.stopPropagation();
          if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
          setBodyOver(true);
        }}
        onDragLeave={() => setBodyOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setBodyOver(false);
          const f = props.ctx.dragFrom();
          if (f) {
            const next = moveNode(props.ctx.root(), f, props.groupPath, children().length);
            if (next !== props.ctx.root()) props.ctx.onChange(next);
          }
          props.ctx.setDragFrom(null);
        }}
      >
        <Show
          when={children().length > 0}
          fallback={
            <p class="py-1 text-xs italic text-ui-muted">
              Empty group — add at least 2 conditions for a meaningful{" "}
              {op() === "and" ? "AND" : "OR"}.
            </p>
          }
        >
          <DropGap ctx={props.ctx} groupPath={props.groupPath} index={0} />
          <Index each={children()}>
            {(_child, i) => (
              <>
                <Show when={i > 0}>
                  <ConnectorChip op={op()} />
                </Show>
                <NodeView ctx={props.ctx} path={[...props.groupPath, i]} />
                <DropGap ctx={props.ctx} groupPath={props.groupPath} index={i + 1} />
              </>
            )}
          </Index>
        </Show>
      </div>

      <div class="pl-3 pt-2">
        <AddBlockDropdown
          label="+ Add condition"
          groupKinds={["and", "or"]}
          onAddLeaf={addLeaf}
          onAddGroup={addGroup}
        />
      </div>
    </div>
  );
};

export default GroupCard;
