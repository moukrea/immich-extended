import { For, Match, Show, Switch, type Component } from "solid-js";
import {
  and,
  or,
  type AndGroup,
  type DateRangeLeaf,
  type FaceRecognitionLeaf,
  type LocationLeaf,
  type MatchExpr,
  type MediaTypeLeaf,
  type NotGroup,
  type OrGroup,
  type PeopleCountLeaf,
  type PersonLeaf,
} from "../../lib/matchTree";
import AddBlockDropdown from "./AddBlockDropdown";
import DateRangeBlock from "./DateRangeBlock";
import FaceRecognitionBlock from "./FaceRecognitionBlock";
import LocationBlock from "./LocationBlock";
import MediaTypeBlock from "./MediaTypeBlock";
import PeopleCountBlock from "./PeopleCountBlock";
import PersonBlock from "./PersonBlock";
import { defaultGroup, defaultLeaf } from "./defaults";

interface NodeProps {
  node: MatchExpr;
  onChange: (next: MatchExpr) => void;
  onRemove: () => void;
  insideNot: boolean;
}

/**
 * Recursive renderer for a MatchExpr subtree. Uses Switch/Match so the kind
 * dispatch tracks reactivity — flipping a node's kind via setState repaints
 * the correct branch without remounting the whole tree.
 */
const NodeRenderer: Component<NodeProps> = (props) => (
  <Switch>
    <Match when={props.node.kind === "leaf" && props.node}>
      {(leaf) => (
        <Switch>
          <Match when={leaf().leaf === "person" && (leaf() as PersonLeaf)}>
            {(p) => (
              <PersonBlock
                leaf={p()}
                onChange={(next) => props.onChange(next)}
                onRemove={() => props.onRemove()}
                insideNot={props.insideNot}
              />
            )}
          </Match>
          <Match
            when={
              leaf().leaf === "people_count" && (leaf() as PeopleCountLeaf)
            }
          >
            {(p) => (
              <PeopleCountBlock
                leaf={p()}
                onChange={(next) => props.onChange(next)}
                onRemove={() => props.onRemove()}
              />
            )}
          </Match>
          <Match
            when={
              leaf().leaf === "face_recognition" &&
              (leaf() as FaceRecognitionLeaf)
            }
          >
            {(p) => (
              <FaceRecognitionBlock
                leaf={p()}
                onChange={(next) => props.onChange(next)}
                onRemove={() => props.onRemove()}
              />
            )}
          </Match>
          <Match
            when={leaf().leaf === "date_range" && (leaf() as DateRangeLeaf)}
          >
            {(p) => (
              <DateRangeBlock
                leaf={p()}
                onChange={(next) => props.onChange(next)}
                onRemove={() => props.onRemove()}
              />
            )}
          </Match>
          <Match when={leaf().leaf === "location" && (leaf() as LocationLeaf)}>
            {(p) => (
              <LocationBlock
                leaf={p()}
                onChange={(next) => props.onChange(next)}
                onRemove={() => props.onRemove()}
              />
            )}
          </Match>
          <Match
            when={leaf().leaf === "media_type" && (leaf() as MediaTypeLeaf)}
          >
            {(p) => (
              <MediaTypeBlock
                leaf={p()}
                onChange={(next) => props.onChange(next)}
                onRemove={() => props.onRemove()}
              />
            )}
          </Match>
        </Switch>
      )}
    </Match>
    <Match
      when={
        props.node.kind === "group" &&
        props.node.op === "not" &&
        (props.node as NotGroup)
      }
    >
      {(g) => (
        <NotGroupNode
          node={g()}
          onChange={(next) => props.onChange(next)}
          onRemove={() => props.onRemove()}
        />
      )}
    </Match>
    <Match
      when={
        props.node.kind === "group" &&
        props.node.op !== "not" &&
        (props.node as AndGroup | OrGroup)
      }
    >
      {(g) => (
        <AndOrGroupNode
          node={g()}
          onChange={(next) => props.onChange(next)}
          onRemove={() => props.onRemove()}
          insideNot={props.insideNot}
        />
      )}
    </Match>
  </Switch>
);

interface AndOrProps {
  node: AndGroup | OrGroup;
  onChange: (next: MatchExpr) => void;
  onRemove: () => void;
  insideNot: boolean;
}

const AndOrGroupNode: Component<AndOrProps> = (props) => {
  const switchOp = (nextOp: "and" | "or") => {
    if (props.node.op === nextOp) return;
    props.onChange(
      nextOp === "and" ? and(props.node.children) : or(props.node.children),
    );
  };
  const replaceChild = (i: number, next: MatchExpr) => {
    const arr = [...props.node.children];
    arr[i] = next;
    props.onChange({ ...props.node, children: arr });
  };
  const removeChild = (i: number) => {
    const arr = props.node.children.filter((_, j) => j !== i);
    props.onChange({ ...props.node, children: arr });
  };
  const appendLeaf = (kind: Parameters<typeof defaultLeaf>[0]) => {
    props.onChange({
      ...props.node,
      children: [...props.node.children, defaultLeaf(kind)],
    });
  };
  const appendGroup = (kind: Parameters<typeof defaultGroup>[0]) => {
    props.onChange({
      ...props.node,
      children: [...props.node.children, defaultGroup(kind)],
    });
  };

  const opLabel = () => (props.node.op === "and" ? "AND" : "OR");
  const opTone = () =>
    props.node.op === "and"
      ? "bg-immich-primary/10 text-immich-primary dark:bg-immich-dark-primary/20 dark:text-immich-dark-primary"
      : "bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-200";

  return (
    <div
      data-testid={`group-${props.node.op}`}
      class="rounded-xl border-2 border-dashed border-ui-border p-3 space-y-2 bg-slate-50/50 dark:bg-gray-900/40"
    >
      <div class="flex items-center justify-between gap-2">
        <div class="flex items-center gap-2">
          <span
            class={`rounded-full px-2 py-0.5 text-[11px] font-bold uppercase tracking-wide ${opTone()}`}
          >
            {opLabel()} group
          </span>
          <div class="inline-flex rounded-md border border-ui-border overflow-hidden text-xs">
            <button
              type="button"
              aria-label="Switch to AND"
              aria-pressed={props.node.op === "and"}
              onClick={() => switchOp("and")}
              class={
                props.node.op === "and"
                  ? "px-2 py-0.5 bg-immich-primary text-white"
                  : "px-2 py-0.5 bg-white dark:bg-gray-700 text-immich-fg dark:text-immich-dark-fg"
              }
            >
              AND
            </button>
            <button
              type="button"
              aria-label="Switch to OR"
              aria-pressed={props.node.op === "or"}
              onClick={() => switchOp("or")}
              class={
                props.node.op === "or"
                  ? "px-2 py-0.5 bg-amber-500 text-white"
                  : "px-2 py-0.5 bg-white dark:bg-gray-700 text-immich-fg dark:text-immich-dark-fg"
              }
            >
              OR
            </button>
          </div>
        </div>
        <button
          type="button"
          onClick={() => props.onRemove()}
          aria-label={`Remove ${opLabel()} group`}
          class="rounded text-xs text-ui-danger hover:underline"
        >
          Remove group
        </button>
      </div>
      <Show when={props.node.children.length === 0}>
        <p class="text-xs text-ui-muted italic">
          Empty group. Add at least 2 conditions for a meaningful{" "}
          {opLabel()}.
        </p>
      </Show>
      <ul class="space-y-2">
        <For each={props.node.children}>
          {(child, i) => (
            <>
              <Show when={i() > 0}>
                <li class="text-center">
                  <span
                    class={`rounded-full px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide ${opTone()}`}
                  >
                    {opLabel()}
                  </span>
                </li>
              </Show>
              <li>
                <NodeRenderer
                  node={child}
                  onChange={(next) => replaceChild(i(), next)}
                  onRemove={() => removeChild(i())}
                  insideNot={props.insideNot}
                />
              </li>
            </>
          )}
        </For>
      </ul>
      <div class="pt-1">
        <AddBlockDropdown onAddLeaf={appendLeaf} onAddGroup={appendGroup} />
      </div>
    </div>
  );
};

interface NotProps {
  node: NotGroup;
  onChange: (next: MatchExpr) => void;
  onRemove: () => void;
}

const NotGroupNode: Component<NotProps> = (props) => {
  const replaceChild = (next: MatchExpr) =>
    props.onChange({ ...props.node, child: next });
  // Removing the child unwraps to an empty AND so the NOT shell doesn't keep
  // a hanging required slot.
  const removeChild = () => props.onChange({ ...props.node, child: and([]) });
  return (
    <div
      data-testid="group-not"
      class="rounded-xl border-2 border-dashed border-rose-300 dark:border-rose-700 p-3 space-y-2 bg-rose-50/40 dark:bg-rose-900/20"
    >
      <div class="flex items-center justify-between gap-2">
        <span class="rounded-full bg-rose-200 dark:bg-rose-800 px-2 py-0.5 text-[11px] font-bold uppercase tracking-wide text-rose-900 dark:text-rose-100">
          NOT
        </span>
        <button
          type="button"
          onClick={() => props.onRemove()}
          aria-label="Remove NOT group"
          class="rounded text-xs text-ui-danger hover:underline"
        >
          Remove group
        </button>
      </div>
      <NodeRenderer
        node={props.node.child}
        onChange={replaceChild}
        onRemove={removeChild}
        insideNot={true}
      />
    </div>
  );
};

export default NodeRenderer;
