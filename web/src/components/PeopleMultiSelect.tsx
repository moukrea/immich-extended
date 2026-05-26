import {
  createMemo,
  createSignal,
  For,
  Show,
  type Component,
} from "solid-js";
import { usePeople } from "./PeopleContext";

interface PeopleMultiSelectProps {
  label: string;
  description?: string;
  value: () => string[];
  onChange: (next: string[]) => void;
  disabled?: boolean;
}

const PeopleMultiSelect: Component<PeopleMultiSelectProps> = (props) => {
  const [query, setQuery] = createSignal("");
  const people = usePeople();

  const all = () => people?.() ?? [];

  const filtered = createMemo(() => {
    const list = all();
    const q = query().trim().toLowerCase();
    if (q.length === 0) return list;
    return list.filter((p) => p.name.toLowerCase().includes(q));
  });

  const selectedSet = createMemo(() => new Set(props.value()));

  const toggle = (id: string) => {
    const current = props.value();
    if (selectedSet().has(id)) {
      props.onChange(current.filter((x) => x !== id));
      return;
    }
    props.onChange([...current, id]);
  };

  const loading = () => people?.loading === true;

  return (
    <div class="mt-4">
      <div class="flex items-baseline justify-between gap-2">
        <p class="text-sm font-medium text-slate-700">{props.label}</p>
        <p class="text-xs text-slate-500">{props.value().length} selected</p>
      </div>
      <Show when={props.description}>
        <p class="mt-0.5 text-xs text-slate-500">{props.description}</p>
      </Show>
      <input
        type="search"
        placeholder="Filter people…"
        value={query()}
        onInput={(e) => setQuery(e.currentTarget.value)}
        disabled={props.disabled}
        class="mt-2 block w-full rounded-md border border-slate-300 px-3 py-1.5 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500 disabled:bg-slate-100 disabled:text-slate-400"
        aria-label={`${props.label} — filter`}
      />
      <Show when={loading()}>
        <p class="mt-2 text-xs text-slate-500">Loading people…</p>
      </Show>
      <Show when={!loading() && all().length === 0}>
        <p class="mt-2 text-xs text-slate-500">
          No people in your Immich library yet.
        </p>
      </Show>
      <Show when={!loading() && all().length > 0 && filtered().length === 0}>
        <p class="mt-2 text-xs text-slate-500">No matches.</p>
      </Show>
      <Show when={!loading() && filtered().length > 0}>
        <ul
          class="mt-2 flex flex-wrap gap-2"
          aria-label={`${props.label} — options`}
        >
          <For each={filtered()}>
            {(person) => {
              const selected = () => selectedSet().has(person.id);
              return (
                <li>
                  <button
                    type="button"
                    aria-pressed={selected()}
                    aria-label={`${selected() ? "Remove" : "Add"} ${person.name} (${props.label})`}
                    disabled={props.disabled}
                    onClick={() => toggle(person.id)}
                    class={
                      selected()
                        ? "inline-flex items-center gap-2 rounded-full border border-indigo-500 bg-indigo-50 px-3 py-1 text-sm text-indigo-900 hover:bg-indigo-100 disabled:opacity-60"
                        : "inline-flex items-center gap-2 rounded-full border border-slate-300 bg-white px-3 py-1 text-sm text-slate-700 hover:bg-slate-100 disabled:opacity-60"
                    }
                  >
                    <img
                      src={person.thumbnail_url}
                      alt=""
                      class="h-6 w-6 rounded-full bg-slate-200 object-cover"
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
    </div>
  );
};

export default PeopleMultiSelect;
