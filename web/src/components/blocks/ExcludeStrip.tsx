// "Always exclude" blacklist lane for the drag-and-drop block builder
// (POSTSHIP-T35, per `docs/design/dnd-block-builder.md` §5.5). A distinct rose
// lane below the main composer that reads as a blacklist: "these people are
// never matched, even if everything else fits."
//
// It is deliberately simpler than the composer — flat, not draggable, not
// selectable. It is presentational over the exclude entries the composer
// derives from the §3.1 top-level partition: each entry is a person the rule
// excludes (a top-level `person{must_exclude}` leaf, or `not(person{includes})`).
// Adding emits `onAddPerson(id)`; removing a chip emits `onRemove(key)`. The
// composer (a later T35 step) owns translating those intents into the canonical
// top-level tree shape (§3.2 recombination).

import { For, Show, createSignal, type Component } from "solid-js";
import { personLabel } from "../../lib/phrases";
import { usePeople } from "../PeopleContext";
import PersonPicker from "./PersonPicker";

export interface ExcludeEntry {
  /** Stable id the composer maps back to a tree node for removal. */
  key: string;
  person_id: string;
}

interface Props {
  entries: () => ExcludeEntry[];
  onAddPerson: (personId: string) => void;
  onRemove: (key: string) => void;
}

const ExcludeStrip: Component<Props> = (props) => {
  const people = usePeople();
  const lookup = (id: string): string | undefined =>
    people?.()?.people.find((p) => p.id === id)?.name;

  const [adding, setAdding] = createSignal(false);

  const add = (id: string) => {
    if (!id) return;
    props.onAddPerson(id);
    setAdding(false);
  };

  return (
    <section
      data-testid="exclude-strip"
      aria-label="Always exclude"
      class="rounded-2xl border border-rose-300/60 bg-rose-50/40 p-3 dark:border-rose-800/60 dark:bg-rose-900/15"
    >
      <h3 class="text-sm font-semibold text-rose-900 dark:text-rose-100">Always exclude</h3>
      <p class="mb-2 text-xs text-rose-800/80 dark:text-rose-200/70">
        These people are never matched, even if everything else fits.
      </p>
      <div class="flex flex-wrap items-center gap-2">
        <For each={props.entries()}>
          {(entry) => (
            <span
              data-testid="exclude-chip"
              class="inline-flex items-center gap-1.5 rounded-full bg-rose-100 px-2.5 py-1 text-xs font-medium text-rose-900 dark:bg-rose-900/40 dark:text-rose-100"
            >
              <span aria-hidden="true">🚫</span>
              <span>{personLabel(entry.person_id, lookup)}</span>
              <button
                type="button"
                aria-label={`Stop excluding ${personLabel(entry.person_id, lookup)}`}
                onClick={() => props.onRemove(entry.key)}
                class="rounded leading-none text-rose-700 hover:text-rose-950 dark:text-rose-300 dark:hover:text-rose-50"
              >
                ✕
              </button>
            </span>
          )}
        </For>

        <Show
          when={adding()}
          fallback={
            <button
              type="button"
              onClick={() => setAdding(true)}
              aria-label="Add a person to always exclude"
              class="inline-flex items-center gap-1 rounded-full border border-dashed border-rose-400/70 px-2.5 py-1 text-xs font-medium text-rose-800 hover:bg-rose-100/60 dark:border-rose-700 dark:text-rose-200 dark:hover:bg-rose-900/30"
            >
              <span aria-hidden="true">🚫</span> + add a person
            </button>
          }
        >
          <div class="w-full max-w-sm rounded-lg border border-rose-300/60 bg-white/70 p-2 dark:border-rose-800/60 dark:bg-immich-dark-gray/70">
            <PersonPicker label="Exclude person" value="" onChange={add} />
            <button
              type="button"
              onClick={() => setAdding(false)}
              class="mt-1 text-xs text-ui-muted hover:underline"
            >
              Cancel
            </button>
          </div>
        </Show>
      </div>
    </section>
  );
};

export default ExcludeStrip;
