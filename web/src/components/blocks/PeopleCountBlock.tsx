import { For, type Component } from "solid-js";
import type { PeopleCountLeaf, PeopleCountOp } from "../../lib/matchTree";
import BlockShell from "./BlockShell";

interface Props {
  leaf: PeopleCountLeaf;
  onChange: (next: PeopleCountLeaf) => void;
  onRemove: () => void;
}

const OP_LABEL: Record<PeopleCountOp, string> = {
  eq: "= equals",
  ne: "≠ not equals",
  lt: "< less than",
  lte: "≤ at most",
  gt: "> more than",
  gte: "≥ at least",
};

const PeopleCountBlock: Component<Props> = (props) => {
  const onOpChange = (op: PeopleCountOp) =>
    props.onChange({ ...props.leaf, op });
  const onValueChange = (raw: string) => {
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) return;
    const rounded = Math.max(0, Math.round(parsed));
    props.onChange({ ...props.leaf, value: rounded });
  };
  return (
    <BlockShell
      title="People count"
      badge="YOLO"
      testid="block-people-count"
      onRemove={props.onRemove}
    >
      <p class="text-xs text-ui-muted dark:text-gray-400">
        Counts visible humans via on-prem YOLO inference. Slower than other
        filters — runs after cheaper conditions pass.
      </p>
      <div class="flex gap-2 items-end">
        <label class="block text-xs font-medium text-immich-fg/70 dark:text-immich-dark-fg/70 flex-1">
          Operator
          <select
            value={props.leaf.op}
            onChange={(e) => onOpChange(e.currentTarget.value as PeopleCountOp)}
            aria-label="People count operator"
            class="mt-1 block w-full rounded-md border border-ui-border bg-white dark:bg-gray-700 px-2 py-1 text-sm text-immich-fg dark:text-immich-dark-fg"
          >
            <For each={Object.entries(OP_LABEL) as [PeopleCountOp, string][]}>
              {([key, label]) => <option value={key}>{label}</option>}
            </For>
          </select>
        </label>
        <label class="block text-xs font-medium text-immich-fg/70 dark:text-immich-dark-fg/70 w-24">
          Value
          <input
            type="number"
            min={0}
            step={1}
            value={props.leaf.value}
            onInput={(e) => onValueChange(e.currentTarget.value)}
            aria-label="People count value"
            class="mt-1 block w-full rounded-md border border-ui-border bg-white dark:bg-gray-700 px-2 py-1 text-sm text-immich-fg dark:text-immich-dark-fg"
          />
        </label>
      </div>
    </BlockShell>
  );
};

export default PeopleCountBlock;
