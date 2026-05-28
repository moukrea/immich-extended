// Recursive renderer for a `MatchExpr` subtree in the drag-and-drop block
// builder (POSTSHIP-T35, per `docs/design/dnd-block-builder.md` §5.2 / §7).
//
// `NodeView` dispatches one tree node by kind: a leaf renders as a `PillCard`,
// an AND/OR group as a `GroupCard`, and a NOT group as either a NOT-wrapped
// `GroupCard` (when it wraps an AND/OR) or a rose NOT pill (when it wraps a
// single leaf). Every node is wrapped in a `[data-block-wrapper]` that is the
// HTML5 drag source — `dragstart` is ignored unless it originated on a
// `[data-drag-handle]` owned by this wrapper (so clicking inline inputs never
// starts a drag), and the wrapper carries the keyboard reorder fallback
// (Move up / Move down, §13) so drag is never the only path.
//
// All structural edits flow through the path-addressed `treeOps`; the node's
// value is re-derived from `ctx.root()` so an edit never remounts an unrelated
// pill (focus-safe). The composer (BlockTreeEditor, a later T35 step) owns the
// `expr` / `selection` / `dragState` signals and passes them in as `ctx`.

import { Match, Show, Switch, createMemo, type Component } from "solid-js";
import type { MatchExpr, MatchLeaf, NotGroup } from "../../lib/matchTree";
import {
  getNode,
  moveNode,
  parentPath,
  pathToKey,
  removeNode,
  replaceNode,
} from "../../lib/treeOps";
import GroupCard from "./GroupCard";
import PillCard from "./PillCard";

/**
 * Shared, ephemeral edit state the recursive renderer reads and writes. The
 * composer constructs this from its `expr` / `selection` / `dragState` signals;
 * tests construct it from plain signals. `onChange` always receives a NEW full
 * root (the path ops resolve against `root()`), keeping the tree single-source.
 */
export interface TreeEditCtx {
  root: () => MatchExpr;
  onChange: (next: MatchExpr) => void;
  isSelected: (key: string) => boolean;
  setSelected: (key: string, on: boolean) => void;
  dragFrom: () => number[] | null;
  setDragFrom: (path: number[] | null) => void;
}

interface Props {
  ctx: TreeEditCtx;
  path: number[];
}

const NodeView: Component<Props> = (props) => {
  let wrapperEl: HTMLDivElement | undefined;

  const node = createMemo(() => getNode(props.ctx.root(), props.path));
  const key = () => pathToKey(props.path);
  const groupOp = (): "and" | "or" | "not" | null => {
    const n = node();
    return n && n.kind === "group" ? n.op : null;
  };

  const apply = (next: MatchExpr) => {
    if (next !== props.ctx.root()) props.ctx.onChange(next);
  };

  // --- selection (the outer node at `path` is the selectable unit) ----------
  const selected = () => props.ctx.isSelected(key());
  const onSelectedChange = (on: boolean) => props.ctx.setSelected(key(), on);

  // --- keyboard reorder fallback (§13) --------------------------------------
  const siblings = () => {
    const parent = getNode(props.ctx.root(), parentPath(props.path));
    if (!parent || parent.kind === "leaf" || parent.op === "not") return null;
    return { count: parent.children.length, idx: props.path[props.path.length - 1]! };
  };
  const canUp = () => {
    const s = siblings();
    return !!s && s.idx > 0;
  };
  const canDown = () => {
    const s = siblings();
    return !!s && s.idx < s.count - 1;
  };
  const moveUp = () => {
    const s = siblings();
    if (!s || s.idx <= 0) return;
    apply(moveNode(props.ctx.root(), props.path, parentPath(props.path), s.idx - 1));
  };
  const moveDown = () => {
    const s = siblings();
    if (!s || s.idx >= s.count - 1) return;
    apply(moveNode(props.ctx.root(), props.path, parentPath(props.path), s.idx + 2));
  };
  const showGutter = () => canUp() || canDown();

  // --- drag source ----------------------------------------------------------
  const dragged = () => {
    const f = props.ctx.dragFrom();
    return f !== null && pathToKey(f) === key();
  };

  const onDragStart = (e: DragEvent) => {
    const target = e.target as HTMLElement | null;
    const handle = target?.closest?.("[data-drag-handle]") as HTMLElement | null;
    // Only a drag that begins on a handle inside THIS wrapper is a real drag.
    if (!handle || !wrapperEl || !wrapperEl.contains(handle)) {
      e.preventDefault();
      return;
    }
    // The handle's nearest wrapper claims the drag; ancestors bail so the path
    // carried matches the grabbed card, not an enclosing group.
    if (handle.closest("[data-block-wrapper]") !== wrapperEl) return;
    if (props.path.length === 0) {
      e.preventDefault(); // the root has no parent to move within
      return;
    }
    if (e.dataTransfer) {
      e.dataTransfer.setData("application/x-block-path", key());
      e.dataTransfer.effectAllowed = "move";
      if (wrapperEl && typeof e.dataTransfer.setDragImage === "function") {
        e.dataTransfer.setDragImage(wrapperEl, 12, 12);
      }
    }
    props.ctx.setDragFrom(props.path);
    e.stopPropagation();
  };
  const onDragEnd = () => props.ctx.setDragFrom(null);

  // --- NOT-group child inspection -------------------------------------------
  const notChild = () => {
    const n = node();
    return n && n.kind === "group" && n.op === "not" ? (n as NotGroup).child : null;
  };
  const notWrapsGroup = () => {
    const c = notChild();
    return !!c && c.kind === "group" && c.op !== "not";
  };

  return (
    <Show when={node()}>
      <div
        ref={wrapperEl}
        data-block-wrapper
        draggable={true}
        onDragStart={onDragStart}
        onDragEnd={onDragEnd}
        class={dragged() ? "opacity-50" : ""}
      >
        <div class="flex items-stretch gap-1">
          <Show when={showGutter()}>
            <div class="flex shrink-0 flex-col justify-center gap-0.5 text-[10px] text-ui-muted">
              <button
                type="button"
                aria-label="Move up"
                disabled={!canUp()}
                onClick={moveUp}
                class="rounded px-1 leading-none hover:bg-slate-100 disabled:opacity-30 dark:hover:bg-gray-700"
              >
                ▲
              </button>
              <button
                type="button"
                aria-label="Move down"
                disabled={!canDown()}
                onClick={moveDown}
                class="rounded px-1 leading-none hover:bg-slate-100 disabled:opacity-30 dark:hover:bg-gray-700"
              >
                ▼
              </button>
            </div>
          </Show>

          <div class="min-w-0 flex-1">
            <Switch>
              <Match when={node()!.kind === "leaf"}>
                <PillCard
                  leaf={node() as MatchLeaf}
                  onChange={(n) => apply(replaceNode(props.ctx.root(), props.path, n))}
                  onRemove={() => apply(removeNode(props.ctx.root(), props.path))}
                  selected={selected()}
                  onSelectedChange={onSelectedChange}
                />
              </Match>

              <Match when={groupOp() !== null && groupOp() !== "not"}>
                <GroupCard
                  ctx={props.ctx}
                  groupPath={props.path}
                  notPath={null}
                  selected={selected()}
                  onSelectedChange={onSelectedChange}
                />
              </Match>

              <Match when={groupOp() === "not"}>
                <Show
                  when={notWrapsGroup()}
                  fallback={
                    <Show
                      when={notChild()?.kind === "leaf"}
                      fallback={<NodeView ctx={props.ctx} path={[...props.path, 0]} />}
                    >
                      <div
                        data-testid="not-leaf-card"
                        class="rounded-2xl border border-l-4 border-rose-300/70 bg-rose-50/40 p-2 dark:border-rose-800/70 dark:bg-rose-900/15"
                      >
                        <span class="mb-1 inline-block rounded-full bg-rose-200 px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-rose-900 dark:bg-rose-800 dark:text-rose-100">
                          NOT
                        </span>
                        <PillCard
                          leaf={notChild() as MatchLeaf}
                          onChange={(n) =>
                            apply(replaceNode(props.ctx.root(), [...props.path, 0], n))
                          }
                          onRemove={() => apply(removeNode(props.ctx.root(), props.path))}
                          selected={selected()}
                          onSelectedChange={onSelectedChange}
                        />
                      </div>
                    </Show>
                  }
                >
                  <GroupCard
                    ctx={props.ctx}
                    groupPath={[...props.path, 0]}
                    notPath={props.path}
                    selected={selected()}
                    onSelectedChange={onSelectedChange}
                  />
                </Show>
              </Match>
            </Switch>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default NodeView;
