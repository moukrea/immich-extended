import { Show, type Component } from "solid-js";
import { and, emptyMatch, type MatchExpr } from "../../lib/matchTree";
import AddBlockDropdown from "./AddBlockDropdown";
import NodeRenderer from "./GroupNode";
import { defaultGroup, defaultLeaf } from "./defaults";

interface Props {
  expr: MatchExpr;
  onChange: (next: MatchExpr) => void;
}

// UI-specific emptiness check: a NOT shell with an empty child is still
// rendered (the user just added it and is about to fill it in). The
// matchTree `isEmpty` helper conservatively treats it as empty because the
// engine evaluates `NOT(emptyMatch)` as a no-op; in the editor we want the
// shell visible so the user can populate it.
function shouldShowEmptyState(expr: MatchExpr): boolean {
  if (expr.kind === "leaf") return false;
  if (expr.op === "not") return false;
  return expr.children.length === 0;
}

/**
 * Root composer for the block-based rule editor. Normalizes the root so the
 * UI always has somewhere to append (an outer AND wrapper). On the wire the
 * single-child AND is emitted as-is — the matchTree serializer matches the
 * Rust impl exactly and a single-child AND parses identically to a bare leaf.
 */
const BlockTreeEditor: Component<Props> = (props) => {
  const onReplace = (next: MatchExpr) => props.onChange(next);

  const addLeafToEmpty = (kind: Parameters<typeof defaultLeaf>[0]) =>
    props.onChange(defaultLeaf(kind));
  const addGroupToEmpty = (kind: Parameters<typeof defaultGroup>[0]) =>
    props.onChange(defaultGroup(kind));

  // When the root is a single leaf and the user clicks "+ Add condition",
  // wrap the leaf into an AND so the next addition becomes a sibling.
  const wrapAndAppendLeaf = (kind: Parameters<typeof defaultLeaf>[0]) => {
    if (props.expr.kind === "leaf") {
      props.onChange(and([props.expr, defaultLeaf(kind)]));
      return;
    }
    addLeafToEmpty(kind);
  };
  const wrapAndAppendGroup = (kind: Parameters<typeof defaultGroup>[0]) => {
    if (props.expr.kind === "leaf") {
      props.onChange(and([props.expr, defaultGroup(kind)]));
      return;
    }
    addGroupToEmpty(kind);
  };

  return (
    <div data-testid="block-tree-editor" class="space-y-3">
      <Show
        when={!shouldShowEmptyState(props.expr)}
        fallback={
          <div class="rounded-xl border-2 border-dashed border-ui-border bg-slate-50/50 dark:bg-gray-900/40 p-4 text-center">
            <p class="text-sm text-ui-muted dark:text-gray-400 mb-3">
              No conditions yet. The rule will match every asset.
            </p>
            <AddBlockDropdown
              label="+ Add condition"
              onAddLeaf={addLeafToEmpty}
              onAddGroup={addGroupToEmpty}
            />
          </div>
        }
      >
        <NodeRenderer
          node={props.expr}
          onChange={onReplace}
          onRemove={() => props.onChange(emptyMatch())}
          insideNot={false}
        />
        <Show when={props.expr.kind === "leaf"}>
          <div class="text-center">
            <AddBlockDropdown
              label="+ Add condition"
              onAddLeaf={wrapAndAppendLeaf}
              onAddGroup={wrapAndAppendGroup}
            />
          </div>
        </Show>
      </Show>
    </div>
  );
};

export default BlockTreeEditor;
