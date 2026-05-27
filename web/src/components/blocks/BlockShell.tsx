import { type Component, type JSX } from "solid-js";

interface Props {
  title: string;
  badge?: string;
  testid: string;
  onRemove: () => void;
  children: JSX.Element;
}

/**
 * Shared card chrome for a tree-builder leaf block. Provides the title row,
 * the badge slot (e.g. mode hint on a Person block), and a Remove button. The
 * body is the leaf-specific form.
 */
const BlockShell: Component<Props> = (props) => (
  <div
    data-testid={props.testid}
    class="rounded-xl border border-ui-border bg-white dark:bg-immich-dark-gray p-3 space-y-2 shadow-sm"
  >
    <div class="flex items-center justify-between gap-2">
      <div class="flex items-center gap-2">
        <span class="text-xs font-semibold uppercase tracking-wide text-immich-fg/70 dark:text-immich-dark-fg/70">
          {props.title}
        </span>
        {props.badge ? (
          <span class="rounded-full bg-slate-200 dark:bg-gray-600 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-immich-fg dark:text-immich-dark-fg">
            {props.badge}
          </span>
        ) : null}
      </div>
      <button
        type="button"
        onClick={() => props.onRemove()}
        aria-label={`Remove ${props.title} block`}
        class="rounded text-xs text-ui-danger hover:underline"
      >
        Remove
      </button>
    </div>
    {props.children}
  </div>
);

export default BlockShell;
