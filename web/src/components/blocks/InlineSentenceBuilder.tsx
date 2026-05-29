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
  Match,
  Show,
  Suspense,
  Switch,
  createEffect,
  createMemo,
  createSignal,
  lazy,
  on,
  onCleanup,
  type Component,
  type JSX,
} from "solid-js";
import {
  serializeMatchExpr,
  type MatchExpr,
  type MatchLeaf,
  type MediaTypeValue,
  type PeopleCountOp,
  type PersonMode,
} from "../../lib/matchTree";
import { normalizeTree } from "../../lib/treeOps";
import { leafSentence, type PersonNameLookup } from "../../lib/phrases";
import {
  locationAreas,
  movePill,
  movePillStep,
  pillLocKey,
  sentenceReadout,
  sentenceToTree,
  treeToSentence,
  type AreaRef,
  type Clause,
  type ClauseMode,
  type Fill,
  type PillLoc,
  type SentenceModel,
} from "../../lib/sentenceModel";
import { defaultLeaf, type AddableLeafKind } from "./defaults";
import AddBlockDropdown from "./AddBlockDropdown";
import { usePeople } from "../PeopleContext";
import PersonPicker from "./PersonPicker";

// The map is the lazy 1054 kB MapLibre chunk; only pulled when a geo Area
// exists (the area blocks below the sentence mount it under a <Suspense>).
const MapPicker = lazy(() => import("../MapPicker"));

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

// Operator labels for the people-count select — symbol + word so the dropdown
// is legible; the at-rest pill phrase (phrases.ts) shows just the symbol.
const OP_SELECT_LABEL: Record<PeopleCountOp, string> = {
  eq: "= equals",
  ne: "≠ not equals",
  lt: "< fewer than",
  lte: "≤ at most",
  gt: "> more than",
  gte: "≥ at least",
};

function isoToInput(iso: string | null): string {
  if (!iso) return "";
  const m = /^(\d{4}-\d{2}-\d{2})/.exec(iso);
  return m ? m[1]! : "";
}

function inputToIso(date: string, endOfDay: boolean): string | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return null;
  return endOfDay ? `${date}T23:59:59Z` : `${date}T00:00:00Z`;
}

function mediaSelectValue(types: MediaTypeValue[]): "photo" | "video" | "both" {
  const hasPhoto = types.includes("photo");
  const hasVideo = types.includes("video");
  if (hasPhoto && hasVideo) return "both";
  if (hasVideo) return "video";
  return "photo";
}

function mediaTypesFromValue(value: string): MediaTypeValue[] {
  if (value === "video") return ["video"];
  if (value === "both") return ["photo", "video"];
  return ["photo"];
}

function clampNonNegInt(raw: string, fallback: number): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return fallback;
  return Math.max(0, Math.round(n));
}

/**
 * Leaves that open an inline popup editor — everything except `location`, whose
 * editor is the numbered map block below the sentence. A location pill is not
 * disabled: clicking it scrolls to (and flashes) its linked Area block.
 */
function isEditableLeaf(leaf: MatchLeaf): boolean {
  return leaf.leaf !== "location";
}

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
  onFocusArea?: () => void;
  // Drag-and-drop (T51) — provided whenever the pill lives in a clause.
  isDragSource?: boolean;
  dragActive?: boolean;
  hovered?: boolean;
  onDragStart?: () => void;
  onDragEnd?: () => void;
  onDragEnter?: () => void;
  onDrop?: () => void;
  onMoveEarlier?: () => void;
  onMoveLater?: () => void;
}> = (props) => {
  const [open, setOpen] = createSignal(false);
  // `draggable` is toggled on only while the grip is held, so dragging never
  // starts from inside the inline editor popup (canonical handle-drag trick).
  const [grabReady, setGrabReady] = createSignal(false);
  const dnd = () => !!props.onDragStart;
  const editable = () => isEditableLeaf(props.leaf);
  const isLocation = () => props.leaf.leaf === "location";
  const atRest = createMemo(() => leafSentence(props.leaf, props.lookup, props.areaNumber));

  return (
    <span
      class="group relative inline-flex items-center"
      data-testid={`pill-${props.leaf.leaf}`}
      draggable={dnd() && grabReady() ? true : undefined}
      onDragStart={(e) => {
        if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
        props.onDragStart?.();
      }}
      onDragEnd={() => {
        setGrabReady(false);
        props.onDragEnd?.();
      }}
      onDragEnter={() => props.onDragEnter?.()}
      onDragOver={(e) => {
        if (props.dragActive) e.preventDefault();
      }}
      onDrop={(e) => {
        e.preventDefault();
        props.onDrop?.();
      }}
      classList={{
        "opacity-50": props.isDragSource,
        "rounded-full ring-2 ring-immich-primary":
          props.dragActive && props.hovered && !props.isDragSource,
      }}
    >
      <span class="inline-flex items-center rounded-full border border-ui-border bg-white py-1 pl-1.5 pr-1 text-sm shadow-sm dark:bg-immich-dark-gray">
        <Show when={dnd()}>
          <button
            type="button"
            data-drag-handle
            aria-label={`Reorder ${atRest()}; use arrow keys to move`}
            title="Drag to reorder, or focus and use arrow keys"
            onMouseDown={() => setGrabReady(true)}
            onMouseUp={() => setGrabReady(false)}
            onKeyDown={(e) => {
              if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
                e.preventDefault();
                props.onMoveEarlier?.();
              } else if (e.key === "ArrowRight" || e.key === "ArrowDown") {
                e.preventDefault();
                props.onMoveLater?.();
              }
            }}
            class="mr-1 cursor-grab select-none rounded px-1 text-ui-muted opacity-30 transition-opacity hover:bg-slate-100 group-hover:opacity-100 focus-visible:opacity-100 dark:hover:bg-gray-700"
          >
            ⠿
          </button>
        </Show>
        <button
          type="button"
          aria-haspopup={editable() ? "true" : undefined}
          aria-expanded={editable() ? open() : undefined}
          onClick={() => {
            if (editable()) setOpen((o) => !o);
            else if (isLocation()) props.onFocusArea?.();
          }}
          class="inline-flex items-center gap-1 font-medium text-immich-fg dark:text-immich-dark-fg"
          classList={{ "cursor-pointer hover:text-immich-primary": editable() || isLocation() }}
        >
          <span>{atRest()}</span>
          <Show when={editable()}>
            <span aria-hidden="true" class="text-ui-muted">
              ▾
            </span>
          </Show>
          <Show when={isLocation()}>
            <span aria-hidden="true" class="text-ui-muted">
              📍
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

      <Show when={open() && editable()}>
        <div
          class="absolute left-0 top-full z-20 mt-1 space-y-2 rounded-xl border border-ui-border bg-white p-3 shadow-lg dark:bg-immich-dark-gray"
          classList={{
            "w-72": props.leaf.leaf === "person",
            "w-64": props.leaf.leaf !== "person",
          }}
        >
          <Switch>
            <Match when={props.leaf.leaf === "person"}>
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
            </Match>

            <Match when={props.leaf.leaf === "people_count"}>
              <label class="block">
                <span class="mb-1 block text-xs font-medium text-ui-muted">People count</span>
                <div class="flex items-center gap-2">
                  <select
                    aria-label="People count operator"
                    class={INLINE_SELECT}
                    value={props.leaf.leaf === "people_count" ? props.leaf.op : "gte"}
                    onChange={(e) => {
                      if (props.leaf.leaf !== "people_count") return;
                      props.onChange({
                        ...props.leaf,
                        op: e.currentTarget.value as PeopleCountOp,
                      });
                    }}
                  >
                    <For each={Object.entries(OP_SELECT_LABEL) as [PeopleCountOp, string][]}>
                      {([key, label]) => <option value={key}>{label}</option>}
                    </For>
                  </select>
                  <input
                    type="number"
                    min={0}
                    step={1}
                    class={`${INLINE_SELECT} w-20`}
                    aria-label="People count value"
                    value={props.leaf.leaf === "people_count" ? props.leaf.value : 0}
                    onInput={(e) => {
                      if (props.leaf.leaf !== "people_count") return;
                      props.onChange({
                        ...props.leaf,
                        value: clampNonNegInt(e.currentTarget.value, props.leaf.value),
                      });
                    }}
                  />
                </div>
              </label>
            </Match>

            <Match when={props.leaf.leaf === "face_recognition"}>
              <span class="block text-xs font-medium text-ui-muted">Face recognition</span>
              <label class="flex items-center gap-2 text-sm text-immich-fg dark:text-immich-dark-fg">
                <input
                  type="checkbox"
                  checked={
                    props.leaf.leaf === "face_recognition"
                      ? !props.leaf.allow_unrecognized
                      : false
                  }
                  aria-label="Require all faces recognized"
                  onChange={(e) => {
                    if (props.leaf.leaf !== "face_recognition") return;
                    props.onChange({
                      ...props.leaf,
                      allow_unrecognized: !e.currentTarget.checked,
                    });
                  }}
                />
                all faces must be recognized
              </label>
              <label class="flex items-center gap-2 text-sm text-immich-fg dark:text-immich-dark-fg">
                <input
                  type="checkbox"
                  checked={
                    props.leaf.leaf === "face_recognition"
                      ? props.leaf.yolo_count_check
                      : false
                  }
                  aria-label="Also reject extra humans (YOLO)"
                  onChange={(e) => {
                    if (props.leaf.leaf !== "face_recognition") return;
                    props.onChange({
                      ...props.leaf,
                      yolo_count_check: e.currentTarget.checked,
                    });
                  }}
                />
                also reject extra humans (YOLO)
              </label>
            </Match>

            <Match when={props.leaf.leaf === "date_range"}>
              <label class="block">
                <span class="mb-1 block text-xs font-medium text-ui-muted">From</span>
                <input
                  type="date"
                  class={`${INLINE_SELECT} w-full`}
                  aria-label="Date from"
                  value={props.leaf.leaf === "date_range" ? isoToInput(props.leaf.from) : ""}
                  onInput={(e) => {
                    if (props.leaf.leaf !== "date_range") return;
                    props.onChange({
                      ...props.leaf,
                      from: inputToIso(e.currentTarget.value, false),
                    });
                  }}
                />
              </label>
              <label class="block">
                <span class="mb-1 block text-xs font-medium text-ui-muted">To</span>
                <input
                  type="date"
                  class={`${INLINE_SELECT} w-full`}
                  aria-label="Date to"
                  value={props.leaf.leaf === "date_range" ? isoToInput(props.leaf.to) : ""}
                  onInput={(e) => {
                    if (props.leaf.leaf !== "date_range") return;
                    props.onChange({
                      ...props.leaf,
                      to: inputToIso(e.currentTarget.value, true),
                    });
                  }}
                />
              </label>
            </Match>

            <Match when={props.leaf.leaf === "media_type"}>
              <label class="block">
                <span class="mb-1 block text-xs font-medium text-ui-muted">Media type</span>
                <select
                  aria-label="Media type"
                  class={`${INLINE_SELECT} w-full`}
                  value={
                    props.leaf.leaf === "media_type"
                      ? mediaSelectValue(props.leaf.types)
                      : "photo"
                  }
                  onChange={(e) => {
                    if (props.leaf.leaf !== "media_type") return;
                    props.onChange({
                      ...props.leaf,
                      types: mediaTypesFromValue(e.currentTarget.value),
                    });
                  }}
                >
                  <option value="photo">photo</option>
                  <option value="video">video</option>
                  <option value="both">photo or video</option>
                </select>
              </label>
            </Match>
          </Switch>
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
  areaBase: number;
  onModeChange: (mode: ClauseMode) => void;
  onPillChange: (index: number, next: MatchLeaf) => void;
  onPillRemove: (index: number) => void;
  onAdd: (kind: AddableLeafKind) => void;
  onFocusArea: (areaNumber: number) => void;
  // Drag-and-drop (T51).
  locOf: (pillIndex: number) => PillLoc;
  dragActive: boolean;
  srcKey: string | null;
  hoverKey: string | null;
  onPillDragStart: (loc: PillLoc) => void;
  onPillDragEnd: () => void;
  onPillDragEnter: (loc: PillLoc) => void;
  onPillDrop: (loc: PillLoc) => void;
  onPillStep: (loc: PillLoc, dir: "earlier" | "later") => void;
}> = (props) => {
  // The "Area N" shown on a location pill = locations in earlier clauses
  // (areaBase) + locations in this clause up to and including this pill.
  const areaNumberAt = (localIndex: number): number | undefined => {
    const pills = props.clause.pills;
    if (pills[localIndex]?.leaf !== "location") return undefined;
    let n = props.areaBase;
    for (let k = 0; k <= localIndex; k++) {
      if (pills[k]?.leaf === "location") n += 1;
    }
    return n;
  };

  // The "+ condition" affordance doubles as the drop zone for the clause end.
  const endLoc = (): PillLoc => props.locOf(props.clause.pills.length);

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
        {(leaf, i) => {
          const loc = (): PillLoc => props.locOf(i);
          const key = (): string => pillLocKey(loc());
          return (
            <>
              <Show when={i > 0}>
                <span class="text-sm font-semibold text-immich-primary">
                  {props.clause.mode === "all" ? "and" : "or"}
                </span>
              </Show>
              <ConditionPill
                leaf={leaf()}
                lookup={props.lookup}
                areaNumber={areaNumberAt(i)}
                onChange={(next) => props.onPillChange(i, next)}
                onRemove={() => props.onPillRemove(i)}
                onFocusArea={() => {
                  const n = areaNumberAt(i);
                  if (n !== undefined) props.onFocusArea(n);
                }}
                isDragSource={props.srcKey === key()}
                dragActive={props.dragActive}
                hovered={props.hoverKey === key()}
                onDragStart={() => props.onPillDragStart(loc())}
                onDragEnd={props.onPillDragEnd}
                onDragEnter={() => props.onPillDragEnter(loc())}
                onDrop={() => props.onPillDrop(loc())}
                onMoveEarlier={() => props.onPillStep(loc(), "earlier")}
                onMoveLater={() => props.onPillStep(loc(), "later")}
              />
            </>
          );
        }}
      </Index>
      <span
        classList={{
          "rounded-full ring-2 ring-immich-primary":
            props.dragActive && props.hoverKey === pillLocKey(endLoc()),
        }}
        onDragEnter={() => props.onPillDragEnter(endLoc())}
        onDragOver={(e) => {
          if (props.dragActive) e.preventDefault();
        }}
        onDrop={(e) => {
          e.preventDefault();
          props.onPillDrop(endLoc());
        }}
      >
        <AddBlockDropdown
          label="+ condition"
          groupKinds={[]}
          triggerClass="inline-flex items-center gap-1 rounded-full border border-dashed border-ui-border px-2.5 py-1 text-sm text-ui-muted hover:border-immich-primary hover:text-immich-primary"
          onAddLeaf={(kind) => props.onAdd(kind)}
          onAddGroup={() => {}}
        />
      </span>
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
  const addPrimaryPill = (kind: AddableLeafKind) => {
    const m = model();
    if (!m) return;
    const pills = [...m.primary.pills, defaultLeaf(kind)];
    commit({ ...m, primary: { ...m.primary, pills } });
  };

  // Except-clause mutations mirror the primary ones, keyed by clause index `i`.
  const addExcept = () => {
    const m = model();
    if (!m) return;
    commit({ ...m, excepts: [...m.excepts, { mode: "all", pills: [] }] });
  };
  const removeExcept = (i: number) => {
    const m = model();
    if (!m) return;
    commit({ ...m, excepts: m.excepts.filter((_, idx) => idx !== i) });
  };
  const setExceptMode = (i: number, mode: ClauseMode) => {
    const m = model();
    if (!m) return;
    const excepts = m.excepts.slice();
    excepts[i] = { ...excepts[i]!, mode };
    commit({ ...m, excepts });
  };
  const changeExceptPill = (i: number, j: number, next: MatchLeaf) => {
    const m = model();
    if (!m) return;
    const excepts = m.excepts.slice();
    const pills = excepts[i]!.pills.slice();
    pills[j] = next;
    excepts[i] = { ...excepts[i]!, pills };
    commit({ ...m, excepts });
  };
  const removeExceptPill = (i: number, j: number) => {
    const m = model();
    if (!m) return;
    const excepts = m.excepts.slice();
    excepts[i] = { ...excepts[i]!, pills: excepts[i]!.pills.filter((_, idx) => idx !== j) };
    commit({ ...m, excepts });
  };
  const addExceptPill = (i: number, kind: AddableLeafKind) => {
    const m = model();
    if (!m) return;
    const excepts = m.excepts.slice();
    excepts[i] = { ...excepts[i]!, pills: [...excepts[i]!.pills, defaultLeaf(kind)] };
    commit({ ...m, excepts });
  };

  // --- Geo areas (L3) ------------------------------------------------------
  // Numbering is derived from the model; the pills, the readout legend, and the
  // map blocks all agree because each scans Location leaves in document order.
  const areas = createMemo(() => {
    const m = model();
    return m ? locationAreas(m) : [];
  });

  const locationCount = (clause: Clause): number =>
    clause.pills.reduce((n, p) => n + (p.leaf === "location" ? 1 : 0), 0);

  // Locations in every clause before except[i] — the base offset for that
  // clause's "Area N" numbering (the primary clause's locations come first).
  const exceptAreaBase = (exceptIndex: number): number => {
    const m = model();
    if (!m) return 0;
    let base = locationCount(m.primary);
    for (let k = 0; k < exceptIndex; k++) base += locationCount(m.excepts[k]!);
    return base;
  };

  const setAreaLocation = (ref: AreaRef, center: [number, number], radiusKm: number) => {
    const leaf: MatchLeaf = { kind: "leaf", leaf: "location", center, radius_km: radiusKm };
    if (ref.clause === "primary") changePrimaryPill(ref.pill, leaf);
    else changeExceptPill(ref.except, ref.pill, leaf);
  };

  // Clicking a "taken in Area N" pill scrolls to and briefly flashes its map
  // block, so the operator can tell which area the inline pill controls.
  const areaEls: (HTMLDivElement | undefined)[] = [];
  const [flashArea, setFlashArea] = createSignal<number | null>(null);
  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  const focusArea = (areaNumber: number) => {
    areaEls[areaNumber - 1]?.scrollIntoView?.({ behavior: "smooth", block: "center" });
    setFlashArea(areaNumber);
    if (flashTimer) clearTimeout(flashTimer);
    flashTimer = setTimeout(() => setFlashArea(null), 1500);
  };
  onCleanup(() => {
    if (flashTimer) clearTimeout(flashTimer);
  });

  // --- Drag-and-drop (L?/T51) ----------------------------------------------
  // The dragged pill's source location and the hovered drop target (a single
  // key, so only one target highlights at a time and it follows the cursor with
  // no dragleave flicker — set on dragenter, cleared only at drag end / drop).
  const [dragSrc, setDragSrc] = createSignal<PillLoc | null>(null);
  const [hoverKey, setHoverKey] = createSignal<string | null>(null);
  const endDrag = () => {
    setDragSrc(null);
    setHoverKey(null);
  };
  const onPillDragEnter = (loc: PillLoc) => {
    if (dragSrc()) setHoverKey(pillLocKey(loc));
  };
  const onPillDrop = (to: PillLoc) => {
    const from = dragSrc();
    const m = model();
    if (from && m && pillLocKey(from) !== pillLocKey(to)) {
      commit(movePill(m, from, to));
    }
    endDrag();
  };
  const onPillStep = (loc: PillLoc, dir: "earlier" | "later") => {
    const m = model();
    if (m) commit(movePillStep(m, loc, dir));
  };
  const srcKey = (): string | null => {
    const s = dragSrc();
    return s ? pillLocKey(s) : null;
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
              areaBase={0}
              onModeChange={setPrimaryMode}
              onPillChange={changePrimaryPill}
              onPillRemove={removePrimaryPill}
              onAdd={addPrimaryPill}
              onFocusArea={focusArea}
              locOf={(pill) => ({ clause: "primary", pill })}
              dragActive={dragSrc() !== null}
              srcKey={srcKey()}
              hoverKey={hoverKey()}
              onPillDragStart={(loc) => setDragSrc(loc)}
              onPillDragEnd={endDrag}
              onPillDragEnter={onPillDragEnter}
              onPillDrop={onPillDrop}
              onPillStep={onPillStep}
            />
          </div>

          <Index each={m().excepts}>
            {(clause, i) => (
              <div class="flex flex-wrap items-center gap-2 border-l-2 border-amber-300 pl-3 dark:border-amber-700/50">
                <span class="text-sm font-medium text-ui-muted dark:text-gray-400">
                  Except if
                </span>
                <ClauseView
                  clause={clause()}
                  lookup={lookup}
                  areaBase={exceptAreaBase(i)}
                  onModeChange={(mode) => setExceptMode(i, mode)}
                  onPillChange={(j, next) => changeExceptPill(i, j, next)}
                  onPillRemove={(j) => removeExceptPill(i, j)}
                  onAdd={(kind) => addExceptPill(i, kind)}
                  onFocusArea={focusArea}
                  locOf={(pill) => ({ clause: "except", except: i, pill })}
                  dragActive={dragSrc() !== null}
                  srcKey={srcKey()}
                  hoverKey={hoverKey()}
                  onPillDragStart={(loc) => setDragSrc(loc)}
                  onPillDragEnd={endDrag}
                  onPillDragEnter={onPillDragEnter}
                  onPillDrop={onPillDrop}
                  onPillStep={onPillStep}
                />
                <button
                  type="button"
                  onClick={() => removeExcept(i)}
                  aria-label={`Remove except clause ${i + 1}`}
                  class="ml-1 rounded-md px-2 py-1 text-xs font-medium text-ui-muted hover:text-ui-danger"
                >
                  ✕ clause
                </button>
              </div>
            )}
          </Index>

          <button
            type="button"
            onClick={addExcept}
            class="inline-flex items-center gap-1 rounded-md border border-dashed border-ui-border px-3 py-1.5 text-xs font-medium text-ui-muted hover:border-immich-primary hover:text-immich-primary"
          >
            + Except clause
          </button>

          <p
            data-testid="sentence-readout"
            aria-live="polite"
            class="rounded-lg bg-slate-50 px-3 py-2 text-sm leading-relaxed text-immich-fg dark:bg-gray-800 dark:text-immich-dark-fg"
          >
            {sentenceReadout(m(), lookup)}
          </p>

          <Show when={areas().length > 0}>
            <div class="space-y-3" data-testid="area-blocks">
              <Index each={areas()}>
                {(entry, i) => (
                  <div
                    ref={(el) => (areaEls[i] = el)}
                    data-testid={`area-block-${i + 1}`}
                    class="rounded-xl border border-ui-border bg-white p-3 shadow-sm transition dark:bg-immich-dark-gray"
                    classList={{ "ring-2 ring-immich-primary": flashArea() === i + 1 }}
                  >
                    <div class="mb-2 flex items-center gap-2">
                      <span class="inline-flex h-6 w-6 items-center justify-center rounded-full bg-immich-primary text-xs font-semibold text-white">
                        {i + 1}
                      </span>
                      <span class="text-sm font-medium text-immich-fg dark:text-immich-dark-fg">
                        Area {i + 1}
                      </span>
                    </div>
                    <Suspense fallback={<p class="text-sm text-ui-muted">Loading map…</p>}>
                      <MapPicker
                        center={entry().leaf.center}
                        radiusKm={entry().leaf.radius_km}
                        onChange={(center, radiusKm) =>
                          setAreaLocation(entry().ref, center, radiusKm)
                        }
                      />
                    </Suspense>
                  </div>
                )}
              </Index>
            </div>
          </Show>
        </div>
      )}
    </Show>
  );
};

export default InlineSentenceBuilder;
