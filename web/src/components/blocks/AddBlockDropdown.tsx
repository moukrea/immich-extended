import { createSignal, For, Show, type Component } from "solid-js";
import {
  GROUP_LABEL,
  LEAF_LABEL,
  type AddableGroupKind,
  type AddableLeafKind,
} from "./defaults";

interface Props {
  label?: string;
  onAddLeaf: (kind: AddableLeafKind) => void;
  onAddGroup: (kind: AddableGroupKind) => void;
}

/**
 * "+ Add block" trigger that pops a flat list of leaf types and group types
 * the user can append to the current group. Closes on selection or on a
 * click outside (handled by the document mousedown listener while open).
 */
const AddBlockDropdown: Component<Props> = (props) => {
  const [open, setOpen] = createSignal(false);
  let rootRef: HTMLDivElement | undefined;

  const closeOnOutside = (event: MouseEvent) => {
    if (!rootRef) return;
    if (!rootRef.contains(event.target as Node)) setOpen(false);
  };

  const toggle = () => {
    const next = !open();
    if (next) document.addEventListener("mousedown", closeOnOutside);
    else document.removeEventListener("mousedown", closeOnOutside);
    setOpen(next);
  };

  const pickLeaf = (kind: AddableLeafKind) => {
    document.removeEventListener("mousedown", closeOnOutside);
    setOpen(false);
    props.onAddLeaf(kind);
  };
  const pickGroup = (kind: AddableGroupKind) => {
    document.removeEventListener("mousedown", closeOnOutside);
    setOpen(false);
    props.onAddGroup(kind);
  };

  return (
    <div class="relative inline-block" ref={rootRef}>
      <button
        type="button"
        onClick={toggle}
        aria-haspopup="menu"
        aria-expanded={open()}
        class="rounded-md border border-dashed border-ui-border bg-white dark:bg-immich-dark-gray px-3 py-1.5 text-xs font-medium text-immich-fg dark:text-immich-dark-fg hover:bg-slate-50 dark:hover:bg-gray-700"
      >
        {props.label ?? "+ Add block"} ▾
      </button>
      <Show when={open()}>
        <div
          role="menu"
          class="absolute left-0 top-full z-10 mt-1 min-w-[180px] rounded-md border border-ui-border bg-white dark:bg-immich-dark-gray shadow-lg"
        >
          <p class="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-ui-muted dark:text-gray-400">
            Condition
          </p>
          <ul class="py-1">
            <For each={Object.entries(LEAF_LABEL) as [AddableLeafKind, string][]}>
              {([key, label]) => (
                <li>
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => pickLeaf(key)}
                    class="block w-full px-3 py-1.5 text-left text-sm text-immich-fg dark:text-immich-dark-fg hover:bg-slate-100 dark:hover:bg-gray-700"
                  >
                    {label}
                  </button>
                </li>
              )}
            </For>
          </ul>
          <p class="border-t border-ui-border px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-ui-muted dark:text-gray-400">
            Group
          </p>
          <ul class="py-1">
            <For each={Object.entries(GROUP_LABEL) as [AddableGroupKind, string][]}>
              {([key, label]) => (
                <li>
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => pickGroup(key)}
                    class="block w-full px-3 py-1.5 text-left text-sm text-immich-fg dark:text-immich-dark-fg hover:bg-slate-100 dark:hover:bg-gray-700"
                  >
                    {label}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>
    </div>
  );
};

export default AddBlockDropdown;
