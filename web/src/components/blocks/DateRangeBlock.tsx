import { type Component } from "solid-js";
import type { DateRangeLeaf } from "../../lib/matchTree";
import BlockShell from "./BlockShell";

interface Props {
  leaf: DateRangeLeaf;
  onChange: (next: DateRangeLeaf) => void;
  onRemove: () => void;
}

function isoToInput(iso: string | null): string {
  if (!iso) return "";
  const match = /^(\d{4}-\d{2}-\d{2})/.exec(iso);
  return match ? match[1]! : "";
}

function inputToIso(date: string, endOfDay: boolean): string | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return null;
  return endOfDay ? `${date}T23:59:59Z` : `${date}T00:00:00Z`;
}

const DateRangeBlock: Component<Props> = (props) => {
  const onFromInput = (date: string) =>
    props.onChange({ ...props.leaf, from: inputToIso(date, false) });
  const onToInput = (date: string) =>
    props.onChange({ ...props.leaf, to: inputToIso(date, true) });
  return (
    <BlockShell
      title="Date range"
      testid="block-date-range"
      onRemove={props.onRemove}
    >
      <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <label class="block text-xs font-medium text-immich-fg/70 dark:text-immich-dark-fg/70">
          From
          <input
            type="date"
            value={isoToInput(props.leaf.from)}
            onInput={(e) => onFromInput(e.currentTarget.value)}
            aria-label="Date range from"
            class="mt-1 block w-full rounded-md border border-ui-border bg-white dark:bg-gray-700 px-2 py-1 text-sm text-immich-fg dark:text-immich-dark-fg"
          />
        </label>
        <label class="block text-xs font-medium text-immich-fg/70 dark:text-immich-dark-fg/70">
          To
          <input
            type="date"
            value={isoToInput(props.leaf.to)}
            onInput={(e) => onToInput(e.currentTarget.value)}
            aria-label="Date range to"
            class="mt-1 block w-full rounded-md border border-ui-border bg-white dark:bg-gray-700 px-2 py-1 text-sm text-immich-fg dark:text-immich-dark-fg"
          />
        </label>
      </div>
    </BlockShell>
  );
};

export default DateRangeBlock;
