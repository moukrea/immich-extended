import { For, type Component } from "solid-js";
import type { PersonLeaf, PersonMode } from "../../lib/matchTree";
import BlockShell from "./BlockShell";
import PersonPicker from "./PersonPicker";

interface Props {
  leaf: PersonLeaf;
  onChange: (next: PersonLeaf) => void;
  onRemove: () => void;
  insideNot: boolean;
}

const MODE_LABEL: Record<PersonMode, string> = {
  must_include: "Must include",
  may_include: "May include",
  must_exclude: "Must exclude",
  includes: "Includes",
};

const PersonBlock: Component<Props> = (props) => {
  const onModeChange = (mode: PersonMode) =>
    props.onChange({ ...props.leaf, mode });
  const onPersonChange = (id: string) =>
    props.onChange({ ...props.leaf, person_id: id });

  // `includes` is the inside-NOT-only mode (per schema validator); hide it
  // outside NOT so a user can't construct an invalid leaf via the UI.
  const modes = (): [PersonMode, string][] => {
    const entries = Object.entries(MODE_LABEL) as [PersonMode, string][];
    return props.insideNot
      ? entries
      : entries.filter(([k]) => k !== "includes");
  };

  return (
    <BlockShell
      title="Person"
      badge={MODE_LABEL[props.leaf.mode]}
      testid="block-person"
      onRemove={props.onRemove}
    >
      <label class="block text-xs font-medium text-immich-fg/70 dark:text-immich-dark-fg/70">
        Mode
        <select
          value={props.leaf.mode}
          onChange={(e) => onModeChange(e.currentTarget.value as PersonMode)}
          aria-label="Person mode"
          class="mt-1 block w-full rounded-md border border-ui-border bg-white dark:bg-gray-700 px-2 py-1 text-sm text-immich-fg dark:text-immich-dark-fg"
        >
          <For each={modes()}>
            {([key, label]) => <option value={key}>{label}</option>}
          </For>
        </select>
      </label>
      <PersonPicker
        label="Person"
        value={props.leaf.person_id}
        onChange={onPersonChange}
      />
    </BlockShell>
  );
};

export default PersonBlock;
