// Inline natural-language rule builder (POSTSHIP cycle 7, T47+).
//
// Replaces the stacked `BlockTreeEditor` with a sentence the operator reads as
// English — "Include to album if Paloma is present and Emeric may be present."
// Each condition is a pill that shows plain language at rest and reveals an
// inline editor on click; a live full-sentence readout is always shown.
//
// Source of truth is a `SentenceModel` (flat: a primary clause + optional
// "except if" clauses), seeded from `props.expr` and re-serialized to a
// `MatchExpr` on every edit. A conservative loader (`treeToSentence`) returns
// `null` for trees that don't fit the flat sentence shape; we then surface the
// Advanced-YAML fallback and touch NOTHING (cycle-7 ABSOLUTE: never corrupt).
//
// T47 wires the shell + the person pill (with the mode dropdown — THE bug fix:
// a second "may be present" person is now possible). Other leaf editors (T48),
// except clauses (T49), geo areas (T50) and drag-drop (T51) extend this file.

import {
  For,
  Index,
  Show,
  createEffect,
  createMemo,
  createSignal,
  on,
  type Component,
  type JSX,
} from "solid-js";
import {
  serializeMatchExpr,
  type MatchExpr,
  type MatchLeaf,
  type PersonMode,
} from "../../lib/matchTree";
import { normalizeTree } from "../../lib/treeOps";
import { leafSentence, type PersonNameLookup } from "../../lib/phrases";
import {
  sentenceReadout,
  sentenceToTree,
  treeToSentence,
  type Clause,
  type ClauseMode,
  type Fill,
  type SentenceModel,
} from "../../lib/sentenceModel";
import { defaultLeaf } from "./defaults";
import { usePeople } from "../PeopleContext";
import PersonPicker from "./PersonPicker";

interface Props {
  expr: MatchExpr;
  onChange: (next: MatchExpr) => void;
  /** Called when the tree can't be shown as a sentence (auto-expand YAML). */
  onRequiresAdvanced?: () => void;
}

const INLINE_SELECT =
  "rounded-md border border-ui-border bg-white px-2 py-1 text-sm text-immich-fg focus:border-immich-primary focus:outline-none focus:ring-1 focus:ring-immich-primary dark:bg-gray-700 dark:text-immich-dark-fg";

const PERSON_MODE_LABEL: Record<"must_include" | "may_include" | "must_exclude", string> = {
  must_include: "is present",
  may_include: "may be present",
  must_exclude: "is not present",
};

// --------------------------------------------------------------------------
// Segmented toggle — used for Include/Exclude (L1) and all/any (L2).
// --------------------------------------------------------------------------

function Segmented<T extends string>(props: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  ariaLabel: string;
}): JSX.Element {
  return (
    <div
      role="group"
      aria-label={props.ariaLabel}
      class="inline-flex rounded-lg border border-ui-border bg-slate-100 p-0.5 dark:bg-gray-800"
    >
      <For each={props.options}>
        {(opt) => (
          <button
            type="button"
            aria-pressed={props.value === opt.value}
            onClick={() => props.onChange(opt.value)}
            class="rounded-md px-2.5 py-1 text-xs font-semibold transition-colors"
            classList={{
              "bg-immich-primary text-white shadow-sm": props.value === opt.value,
              "text-ui-muted hover:text-immich-fg dark:hover:text-immich-dark-fg":
                props.value !== opt.value,
            }}
          >
            {opt.label}
          </button>
        )}
      </For>
    </div>
  );
}

// --------------------------------------------------------------------------
// ConditionPill — one leaf, at-rest phrase + (person) inline editor.
// --------------------------------------------------------------------------

const ConditionPill: Component<{
  leaf: MatchLeaf;
  lookup: PersonNameLookup;
  areaNumber?: number;
  onChange: (next: MatchLeaf) => void;
  onRemove: () => void;
}> = (props) => {
  const [open, setOpen] = createSignal(false);
  const isPerson = () => props.leaf.leaf === "person";
  const atRest = createMemo(() => leafSentence(props.leaf, props.lookup, props.areaNumber));

  return (
    <span class="relative inline-flex items-center" data-testid={`pill-${props.leaf.leaf}`}>
      <span class="inline-flex items-center rounded-full border border-ui-border bg-white py-1 pl-3 pr-1 text-sm shadow-sm dark:bg-immich-dark-gray">
        <button
          type="button"
          disabled={!isPerson()}
          aria-haspopup={isPerson() ? "true" : undefined}
          aria-expanded={isPerson() ? open() : undefined}
          onClick={() => isPerson() && setOpen((o) => !o)}
          class="inline-flex items-center gap-1 font-medium text-immich-fg dark:text-immich-dark-fg"
          classList={{ "cursor-pointer hover:text-immich-primary": isPerson() }}
        >
          <span>{atRest()}</span>
          <Show when={isPerson()}>
            <span aria-hidden="true" class="text-ui-muted">
              ▾
            </span>
          </Show>
        </button>
        <button
          type="button"
          onClick={() => props.onRemove()}
          aria-label={`Remove condition: ${atRest()}`}
          class="ml-1 rounded-full px-1 text-ui-muted hover:text-ui-danger"
        >
          ✕
        </button>
      </span>

      <Show when={open() && props.leaf.leaf === "person"}>
        <div class="absolute left-0 top-full z-20 mt-1 w-72 space-y-2 rounded-xl border border-ui-border bg-white p-3 shadow-lg dark:bg-immich-dark-gray">
          <label class="block">
            <span class="mb-1 block text-xs font-medium text-ui-muted">Condition</span>
            <select
              aria-label="Person condition mode"
              class={`${INLINE_SELECT} w-full`}
              value={props.leaf.leaf === "person" ? props.leaf.mode : "must_include"}
              onChange={(e) => {
                if (props.leaf.leaf !== "person") return;
                props.onChange({
                  ...props.leaf,
                  mode: e.currentTarget.value as PersonMode,
                });
              }}
            >
              <For
                each={
                  Object.entries(PERSON_MODE_LABEL) as [
                    "must_include" | "may_include" | "must_exclude",
                    string,
                  ][]
                }
              >
                {([value, label]) => <option value={value}>{label}</option>}
              </For>
            </select>
          </label>
          <PersonPicker
            label="Person"
            value={props.leaf.leaf === "person" ? props.leaf.person_id : ""}
            onChange={(id) => {
              if (props.leaf.leaf !== "person") return;
              props.onChange({ ...props.leaf, person_id: id });
            }}
          />
        </div>
      </Show>
    </span>
  );
};

// --------------------------------------------------------------------------
// ClauseView — the all/any toggle + inline pills + "+ condition".
// --------------------------------------------------------------------------

const ClauseView: Component<{
  clause: Clause;
  lookup: PersonNameLookup;
  onModeChange: (mode: ClauseMode) => void;
  onPillChange: (index: number, next: MatchLeaf) => void;
  onPillRemove: (index: number) => void;
  onAddPerson: () => void;
}> = (props) => {
  return (
    <div class="flex flex-wrap items-center gap-x-2 gap-y-2">
      <Show when={props.clause.pills.length >= 2}>
        <Segmented
          ariaLabel="Match mode"
          value={props.clause.mode}
          options={[
            { value: "all", label: "all of" },
            { value: "any", label: "any of" },
          ]}
          onChange={props.onModeChange}
        />
      </Show>
      <Index each={props.clause.pills}>
        {(leaf, i) => (
          <>
            <Show when={i > 0}>
              <span class="text-sm font-semibold text-immich-primary">
                {props.clause.mode === "all" ? "and" : "or"}
              </span>
            </Show>
            <ConditionPill
              leaf={leaf()}
              lookup={props.lookup}
              onChange={(next) => props.onPillChange(i, next)}
              onRemove={() => props.onPillRemove(i)}
            />
          </>
        )}
      </Index>
      <button
        type="button"
        onClick={() => props.onAddPerson()}
        aria-label="Add condition"
        class="inline-flex items-center gap-1 rounded-full border border-dashed border-ui-border px-2.5 py-1 text-sm text-ui-muted hover:border-immich-primary hover:text-immich-primary"
      >
        <span aria-hidden="true">+</span> condition
      </button>
    </div>
  );
};

// --------------------------------------------------------------------------
// InlineSentenceBuilder.
// --------------------------------------------------------------------------

const InlineSentenceBuilder: Component<Props> = (props) => {
  const people = usePeople();
  const lookup: PersonNameLookup = (id) =>
    people?.()?.people.find((p) => p.id === id)?.name;

  const [model, setModel] = createSignal<SentenceModel | null>(null);

  const canonical = (expr: MatchExpr): string =>
    JSON.stringify(serializeMatchExpr(normalizeTree(expr)));

  // Re-seed the model when `props.expr` changes from OUTSIDE (e.g. the Advanced
  // YAML panel). Our own commits echo back through `onChange`; the guard skips
  // those so local UI state (open pickers, single-pill clause mode) survives.
  createEffect(
    on(
      () => props.expr,
      (expr) => {
        const current = model();
        if (current && canonical(sentenceToTree(current)) === canonical(expr)) {
          return;
        }
        setModel(treeToSentence(expr));
      },
    ),
  );

  // Auto-expand the Advanced panel when the tree can't be shown as a sentence.
  createEffect(() => {
    if (model() === null) props.onRequiresAdvanced?.();
  });

  const commit = (next: SentenceModel) => {
    setModel(next);
    props.onChange(normalizeTree(sentenceToTree(next)));
  };

  const setFill = (fill: Fill) => {
    const m = model();
    if (m) commit({ ...m, fill });
  };
  const setPrimaryMode = (mode: ClauseMode) => {
    const m = model();
    if (m) commit({ ...m, primary: { ...m.primary, mode } });
  };
  const changePrimaryPill = (index: number, next: MatchLeaf) => {
    const m = model();
    if (!m) return;
    const pills = m.primary.pills.slice();
    pills[index] = next;
    commit({ ...m, primary: { ...m.primary, pills } });
  };
  const removePrimaryPill = (index: number) => {
    const m = model();
    if (!m) return;
    const pills = m.primary.pills.filter((_, i) => i !== index);
    commit({ ...m, primary: { ...m.primary, pills } });
  };
  const addPrimaryPerson = () => {
    const m = model();
    if (!m) return;
    const pills = [...m.primary.pills, defaultLeaf("person")];
    commit({ ...m, primary: { ...m.primary, pills } });
  };

  return (
    <Show
      when={model()}
      fallback={
        <div
          role="status"
          class="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-700/50 dark:bg-amber-900/20 dark:text-amber-100"
        >
          This rule uses advanced logic that the sentence builder can't show.
          Edit it in the Advanced (YAML) panel below.
        </div>
      }
    >
      {(m) => (
        <div class="space-y-4">
          <div class="flex flex-wrap items-center gap-2">
            <Segmented
              ariaLabel="Include or exclude"
              value={m().fill}
              options={[
                { value: "include", label: "Include" },
                { value: "exclude", label: "Exclude" },
              ]}
              onChange={setFill}
            />
            <span class="text-sm text-ui-muted dark:text-gray-400">
              {m().fill === "include" ? "to album if" : "from album if"}
            </span>
            <ClauseView
              clause={m().primary}
              lookup={lookup}
              onModeChange={setPrimaryMode}
              onPillChange={changePrimaryPill}
              onPillRemove={removePrimaryPill}
              onAddPerson={addPrimaryPerson}
            />
          </div>

          <p
            data-testid="sentence-readout"
            aria-live="polite"
            class="rounded-lg bg-slate-50 px-3 py-2 text-sm leading-relaxed text-immich-fg dark:bg-gray-800 dark:text-immich-dark-fg"
          >
            {sentenceReadout(m(), lookup)}
          </p>
        </div>
      )}
    </Show>
  );
};

export default InlineSentenceBuilder;
