// A single leaf condition, rendered as a phrase pill-card for the drag-and-drop
// block builder (POSTSHIP-T35, per `docs/design/dnd-block-builder.md` §5.1).
//
// The card reads as English ("Paloma is present", "people count = 1") with the
// variable parts as inline editors. The icon comes from `phrases.ts` (the
// single wording source, also used for the read-out aria-label via
// `leafPhraseText`); the visible phrase is laid out per leaf type with the
// connective wording matching that module and the controls inlined directly so
// they mount once and never remount on edit — typing keeps focus while each
// control's value reads `props.leaf` reactively.
//
// `face_recognition` has no inline controls (two checkboxes instead);
// `date_range` always shows both date inputs so an empty range stays editable.
//
// Selection + drag are owned by the composer (later T35 steps); this card only
// renders the selection checkbox and the drag handle (`data-drag-handle`). The
// surrounding NodeView wires the actual HTML5 drag.

import {
  For,
  Match,
  Show,
  Suspense,
  Switch,
  createMemo,
  createSignal,
  lazy,
  type Component,
} from "solid-js";
import type {
  MatchLeaf,
  MediaTypeValue,
  PeopleCountOp,
} from "../../lib/matchTree";
import { formatLatLng, leafPhrase, leafPhraseText, personLabel } from "../../lib/phrases";
import { usePeople } from "../PeopleContext";
import PersonPicker from "./PersonPicker";

const MapPicker = lazy(() => import("../MapPicker"));

interface Props {
  leaf: MatchLeaf;
  onChange: (next: MatchLeaf) => void;
  onRemove: () => void;
  selected?: boolean;
  onSelectedChange?: (next: boolean) => void;
}

const INLINE_CTL =
  "rounded-md border border-ui-border bg-white dark:bg-gray-700 px-1.5 py-0.5 text-sm text-immich-fg dark:text-immich-dark-fg focus:border-immich-primary focus:outline-none focus:ring-1 focus:ring-immich-primary";

// Operator labels for the people-count select — symbol + word so the dropdown
// is legible; the collapsed phrase elsewhere shows just the symbol.
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

const PillCard: Component<Props> = (props) => {
  const people = usePeople();
  const lookup = (id: string): string | undefined =>
    people?.()?.people.find((p) => p.id === id)?.name;

  const phrase = createMemo(() => leafPhrase(props.leaf, lookup));
  const ariaPhrase = createMemo(() => leafPhraseText(props.leaf, lookup));

  const [personOpen, setPersonOpen] = createSignal(false);
  const [mapOpen, setMapOpen] = createSignal(false);

  // Verb wording mirrors phrases.ts §11 (kept here so the verb is plain text,
  // not an editable control — mode is chosen at add-time / via NOT-wrapping).
  const personVerb = (): string => {
    if (props.leaf.leaf !== "person") return "";
    switch (props.leaf.mode) {
      case "must_include":
        return "is present";
      case "may_include":
        return "may be present";
      case "includes":
        return "appears";
      case "must_exclude":
        return "is excluded";
    }
  };

  return (
    <div
      data-testid={`pill-${props.leaf.leaf}`}
      class={`rounded-xl border bg-white px-3 py-2 shadow-sm dark:bg-immich-dark-gray ${
        props.selected
          ? "border-immich-primary ring-2 ring-immich-primary"
          : "border-ui-border"
      }`}
    >
      <div class="flex items-start gap-2">
        <input
          type="checkbox"
          class="mt-1.5 shrink-0"
          checked={props.selected ?? false}
          aria-label={`Select condition: ${ariaPhrase()}`}
          onChange={(e) => props.onSelectedChange?.(e.currentTarget.checked)}
        />
        <span
          data-drag-handle
          aria-label="Drag to reorder"
          title="Drag to reorder"
          class="mt-1 shrink-0 cursor-grab select-none text-ui-muted"
        >
          ⠿
        </span>
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-1.5 text-sm text-immich-fg dark:text-immich-dark-fg">
            <span aria-hidden="true">{phrase().icon}</span>
            <Switch>
              <Match when={props.leaf.leaf === "person"}>
                <button
                  type="button"
                  aria-haspopup="true"
                  aria-expanded={personOpen()}
                  aria-label="Choose person"
                  onClick={() => setPersonOpen((o) => !o)}
                  class="inline-flex items-center gap-1 rounded-md border border-ui-border bg-white px-1.5 py-0.5 text-sm font-semibold text-immich-fg hover:bg-slate-50 dark:bg-gray-700 dark:text-immich-dark-fg dark:hover:bg-gray-600"
                >
                  <span>
                    {props.leaf.leaf === "person" && props.leaf.person_id
                      ? personLabel(props.leaf.person_id, lookup)
                      : "pick a person"}
                  </span>
                  <span aria-hidden="true">▾</span>
                </button>
                <span>{personVerb()}</span>
              </Match>

              <Match when={props.leaf.leaf === "people_count"}>
                <span>people count</span>
                <select
                  class={INLINE_CTL}
                  aria-label="People count operator"
                  value={
                    props.leaf.leaf === "people_count" ? props.leaf.op : "gte"
                  }
                  onChange={(e) => {
                    if (props.leaf.leaf !== "people_count") return;
                    props.onChange({
                      ...props.leaf,
                      op: e.currentTarget.value as PeopleCountOp,
                    });
                  }}
                >
                  <For
                    each={
                      Object.entries(OP_SELECT_LABEL) as [
                        PeopleCountOp,
                        string,
                      ][]
                    }
                  >
                    {([key, label]) => <option value={key}>{label}</option>}
                  </For>
                </select>
                <input
                  type="number"
                  min={0}
                  step={1}
                  class={`${INLINE_CTL} w-16`}
                  aria-label="People count value"
                  value={
                    props.leaf.leaf === "people_count" ? props.leaf.value : 0
                  }
                  onInput={(e) => {
                    if (props.leaf.leaf !== "people_count") return;
                    props.onChange({
                      ...props.leaf,
                      value: clampNonNegInt(
                        e.currentTarget.value,
                        props.leaf.value,
                      ),
                    });
                  }}
                />
              </Match>

              <Match when={props.leaf.leaf === "face_recognition"}>
                <div class="flex flex-wrap items-center gap-x-4 gap-y-1">
                  <label class="inline-flex items-center gap-1.5">
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
                  <label class="inline-flex items-center gap-1.5">
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
                </div>
              </Match>

              <Match when={props.leaf.leaf === "date_range"}>
                <span>taken from</span>
                <input
                  type="date"
                  class={INLINE_CTL}
                  aria-label="Date from"
                  value={
                    props.leaf.leaf === "date_range"
                      ? isoToInput(props.leaf.from)
                      : ""
                  }
                  onInput={(e) => {
                    if (props.leaf.leaf !== "date_range") return;
                    props.onChange({
                      ...props.leaf,
                      from: inputToIso(e.currentTarget.value, false),
                    });
                  }}
                />
                <span>to</span>
                <input
                  type="date"
                  class={INLINE_CTL}
                  aria-label="Date to"
                  value={
                    props.leaf.leaf === "date_range"
                      ? isoToInput(props.leaf.to)
                      : ""
                  }
                  onInput={(e) => {
                    if (props.leaf.leaf !== "date_range") return;
                    props.onChange({
                      ...props.leaf,
                      to: inputToIso(e.currentTarget.value, true),
                    });
                  }}
                />
              </Match>

              <Match when={props.leaf.leaf === "location"}>
                <span>within</span>
                <input
                  type="number"
                  min={0}
                  step={1}
                  class={`${INLINE_CTL} w-16`}
                  aria-label="Location radius (km)"
                  value={
                    props.leaf.leaf === "location" ? props.leaf.radius_km : 0
                  }
                  onInput={(e) => {
                    if (props.leaf.leaf !== "location") return;
                    props.onChange({
                      ...props.leaf,
                      radius_km: clampNonNegInt(
                        e.currentTarget.value,
                        props.leaf.radius_km,
                      ),
                    });
                  }}
                />
                <span>
                  km of{" "}
                  {props.leaf.leaf === "location"
                    ? formatLatLng(props.leaf.center)
                    : ""}
                </span>
                <button
                  type="button"
                  aria-expanded={mapOpen()}
                  onClick={() => setMapOpen((o) => !o)}
                  class="rounded-md border border-ui-border px-1.5 py-0.5 text-xs text-immich-fg hover:bg-slate-50 dark:text-immich-dark-fg dark:hover:bg-gray-700"
                >
                  {mapOpen() ? "Hide map ▴" : "Map ▾"}
                </button>
              </Match>

              <Match when={props.leaf.leaf === "media_type"}>
                <span>is a</span>
                <select
                  class={INLINE_CTL}
                  aria-label="Media type"
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
              </Match>
            </Switch>
          </div>

          <Show when={props.leaf.leaf === "person" && personOpen()}>
            <div class="mt-2">
              <PersonPicker
                label="Person"
                value={props.leaf.leaf === "person" ? props.leaf.person_id : ""}
                onChange={(id) => {
                  if (props.leaf.leaf !== "person") return;
                  props.onChange({ ...props.leaf, person_id: id });
                  setPersonOpen(false);
                }}
              />
            </div>
          </Show>

          <Show when={props.leaf.leaf === "location" && mapOpen()}>
            <div data-testid="pill-location-map" class="mt-2">
              <Suspense
                fallback={<p class="text-sm text-ui-muted">Loading map…</p>}
              >
                <MapPicker
                  center={
                    props.leaf.leaf === "location" ? props.leaf.center : [0, 0]
                  }
                  radiusKm={
                    props.leaf.leaf === "location" ? props.leaf.radius_km : 0
                  }
                  onChange={(center, radiusKm) => {
                    if (props.leaf.leaf !== "location") return;
                    props.onChange({
                      ...props.leaf,
                      center,
                      radius_km: radiusKm,
                    });
                  }}
                />
              </Suspense>
            </div>
          </Show>
        </div>

        <button
          type="button"
          onClick={() => props.onRemove()}
          aria-label={`Remove condition: ${ariaPhrase()}`}
          class="mt-0.5 shrink-0 rounded px-1 text-ui-muted hover:text-ui-danger"
        >
          ✕
        </button>
      </div>
    </div>
  );
};

export default PillCard;
