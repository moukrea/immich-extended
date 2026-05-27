import { createMemo, createSignal, For, Show, type Component } from "solid-js";
import { A } from "@solidjs/router";
import { usePeople } from "../PeopleContext";

interface Props {
  value: string;
  onChange: (id: string) => void;
  label: string;
}

/**
 * Single-person picker for a Person block in the tree builder. Reads the same
 * PeopleContext resource as PeopleMultiSelect so a page with multiple Person
 * blocks pays one /api/v1/me/people round-trip.
 */
const PersonPicker: Component<Props> = (props) => {
  const [query, setQuery] = createSignal("");
  const people = usePeople();
  const listing = () => people?.();
  const all = () => listing()?.people ?? [];
  const noImmichKey = () => listing()?.noImmichKey === true;
  const loading = () => people?.loading === true;

  const filtered = createMemo(() => {
    const q = query().trim().toLowerCase();
    const list = all();
    if (q.length === 0) return list.slice(0, 50);
    return list.filter((p) => p.name.toLowerCase().includes(q)).slice(0, 50);
  });

  const currentName = createMemo(() => {
    const id = props.value;
    if (!id) return null;
    return all().find((p) => p.id === id)?.name ?? null;
  });

  return (
    <div class="space-y-1.5">
      <Show
        when={!loading() && noImmichKey()}
        fallback={
          <>
            <Show when={props.value && currentName()}>
              <p
                class="text-xs text-immich-fg dark:text-immich-dark-fg"
                aria-live="polite"
              >
                Selected: <span class="font-semibold">{currentName()}</span>
              </p>
            </Show>
            <Show when={props.value && !currentName()}>
              <p
                class="text-xs text-amber-700 dark:text-amber-300"
                aria-live="polite"
              >
                Selected: <code>{props.value}</code> (not in current library)
              </p>
            </Show>
            <input
              type="search"
              placeholder="Filter people…"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              aria-label={`${props.label} — filter`}
              class="block w-full rounded-md border border-ui-border bg-white dark:bg-gray-700 px-2 py-1 text-sm text-immich-fg dark:text-immich-dark-fg focus:border-immich-primary focus:outline-none focus:ring-1 focus:ring-immich-primary"
            />
            <Show when={loading()}>
              <p class="text-xs text-ui-muted">Loading people…</p>
            </Show>
            <Show when={!loading() && all().length === 0}>
              <p class="text-xs text-ui-muted">
                No people in your Immich library yet.
              </p>
            </Show>
            <Show when={!loading() && filtered().length > 0}>
              <ul
                class="flex flex-wrap gap-1.5 max-h-40 overflow-y-auto"
                aria-label={`${props.label} — options`}
              >
                <For each={filtered()}>
                  {(person) => {
                    const selected = () => props.value === person.id;
                    return (
                      <li>
                        <button
                          type="button"
                          aria-pressed={selected()}
                          aria-label={`${selected() ? "Currently selected" : "Pick"} ${person.name}`}
                          onClick={() => props.onChange(person.id)}
                          class={
                            selected()
                              ? "inline-flex items-center gap-1.5 rounded-full border border-immich-primary bg-immich-primary/10 px-2 py-0.5 text-xs text-immich-primary dark:text-immich-dark-primary"
                              : "inline-flex items-center gap-1.5 rounded-full border border-ui-border bg-white dark:bg-gray-700 px-2 py-0.5 text-xs text-immich-fg dark:text-immich-dark-fg hover:bg-slate-100 dark:hover:bg-gray-600"
                          }
                        >
                          <img
                            src={person.thumbnail_url}
                            alt=""
                            class="h-5 w-5 rounded-full bg-slate-200 object-cover"
                            loading="lazy"
                          />
                          <span>{person.name}</span>
                        </button>
                      </li>
                    );
                  }}
                </For>
              </ul>
            </Show>
          </>
        }
      >
        <p
          class="rounded-md border border-amber-300 bg-amber-50 px-2 py-1 text-xs text-amber-900 dark:bg-amber-900/30 dark:text-amber-100"
          role="status"
        >
          Connect your Immich account at{" "}
          <A href="/me" class="font-semibold underline">
            Settings
          </A>{" "}
          to pick people.
        </p>
      </Show>
    </div>
  );
};

export default PersonPicker;
