// Leaf → human phrase rendering for the drag-and-drop block builder.
//
// Each leaf condition renders as a pill-card that reads as English ("Paloma is
// present", "people count = 1", "within 60 km of (48.857, 2.352)"). This module
// is the single source of that wording (per `docs/design/dnd-block-builder.md`
// §11) — pure, framework-agnostic, and unit-tested independent of rendering.
//
// `leafPhrase` returns an icon + an ordered list of `PhrasePart`s. A part is
// either static `text` or a `control` slot the `PillCard` fills with an inline
// editor (`<select>`/`<input>`). The `display` on a control is its read-out text
// — used for `phraseText` (aria-labels, collapsed views, tests) and as the
// fallback when no editor is mounted. `face_recognition` is pure text: its
// toggles are checkboxes the PillCard renders from the leaf directly, so it
// carries no control slots here.

import type { MatchLeaf, MediaTypeValue, PeopleCountOp } from "./matchTree";

// --------------------------------------------------------------------------
// Shapes.
// --------------------------------------------------------------------------

/** Which inline editor the PillCard mounts for a `control` part. */
export type ControlSlot =
  | "person"
  | "people_count_op"
  | "people_count_value"
  | "date_from"
  | "date_to"
  | "location_radius"
  | "media_type";

export type PhrasePart =
  | { kind: "text"; text: string }
  | { kind: "control"; control: ControlSlot; display: string };

export interface LeafPhrase {
  icon: string;
  parts: PhrasePart[];
}

/** Resolve a person id to a display name; `undefined` when not yet loaded. */
export type PersonNameLookup = (personId: string) => string | undefined;

// --------------------------------------------------------------------------
// Operator symbols (shared with the people-count pill control).
// --------------------------------------------------------------------------

export const OP_SYMBOL: Record<PeopleCountOp, string> = {
  eq: "=",
  ne: "≠",
  lt: "<",
  lte: "≤",
  gt: ">",
  gte: "≥",
};

export function opSymbol(op: PeopleCountOp): string {
  return OP_SYMBOL[op];
}

// --------------------------------------------------------------------------
// Value formatting.
// --------------------------------------------------------------------------

/** Short, stable label for a person id — name if known, else a short id. */
export function personLabel(personId: string, lookup: PersonNameLookup): string {
  const name = lookup(personId);
  if (name && name.length > 0) return name;
  return personId.slice(0, 8);
}

/** Trim a coordinate to at most 4 decimals without trailing zeros. */
function formatCoord(n: number): string {
  return String(Number(n.toFixed(4)));
}

export function formatLatLng(center: [number, number]): string {
  return `(${formatCoord(center[0])}, ${formatCoord(center[1])})`;
}

export function mediaTypesLabel(types: MediaTypeValue[]): string {
  const hasPhoto = types.includes("photo");
  const hasVideo = types.includes("video");
  if (hasPhoto && hasVideo) return "photo or video";
  if (hasPhoto) return "photo";
  if (hasVideo) return "video";
  return "photo or video";
}

// --------------------------------------------------------------------------
// Part builders.
// --------------------------------------------------------------------------

function text(t: string): PhrasePart {
  return { kind: "text", text: t };
}

function control(slot: ControlSlot, display: string): PhrasePart {
  return { kind: "control", control: slot, display };
}

// --------------------------------------------------------------------------
// leafPhrase — the wording table from §11.
// --------------------------------------------------------------------------

export function leafPhrase(leaf: MatchLeaf, lookup: PersonNameLookup): LeafPhrase {
  switch (leaf.leaf) {
    case "person": {
      const name = personLabel(leaf.person_id, lookup);
      const pill = control("person", name);
      switch (leaf.mode) {
        case "must_include":
          return { icon: "👤", parts: [pill, text("is present")] };
        case "may_include":
          return { icon: "👤", parts: [pill, text("may be present")] };
        case "includes":
          return { icon: "👤", parts: [pill, text("appears")] };
        case "must_exclude":
          // Only renders in the Always-exclude strip; worded as the blacklist.
          return { icon: "🚫", parts: [text("never"), pill] };
      }
      return { icon: "👤", parts: [pill] };
    }

    case "people_count":
      return {
        icon: "🔢",
        parts: [
          text("people count"),
          control("people_count_op", opSymbol(leaf.op)),
          control("people_count_value", String(leaf.value)),
        ],
      };

    case "face_recognition": {
      if (!leaf.allow_unrecognized) {
        const phrase = leaf.yolo_count_check
          ? "all faces must be recognized · also reject extra humans (YOLO)"
          : "all faces must be recognized";
        return { icon: "🙂", parts: [text(phrase)] };
      }
      if (leaf.yolo_count_check) {
        return { icon: "🙂", parts: [text("no unidentified extra humans (YOLO)")] };
      }
      return { icon: "🙂", parts: [text("unrecognized faces allowed")] };
    }

    case "date_range": {
      const hasFrom = leaf.from !== null;
      const hasTo = leaf.to !== null;
      if (hasFrom && hasTo) {
        return {
          icon: "📅",
          parts: [
            text("taken from"),
            control("date_from", leaf.from ?? ""),
            text("to"),
            control("date_to", leaf.to ?? ""),
          ],
        };
      }
      if (hasFrom) {
        return { icon: "📅", parts: [text("taken after"), control("date_from", leaf.from ?? "")] };
      }
      if (hasTo) {
        return { icon: "📅", parts: [text("taken before"), control("date_to", leaf.to ?? "")] };
      }
      return { icon: "📅", parts: [text("taken on any date")] };
    }

    case "location":
      return {
        icon: "📍",
        parts: [
          text("within"),
          control("location_radius", String(leaf.radius_km)),
          text(`km of ${formatLatLng(leaf.center)}`),
        ],
      };

    case "media_type":
      return {
        icon: "🎞",
        parts: [text("is a"), control("media_type", mediaTypesLabel(leaf.types))],
      };
  }
}

// --------------------------------------------------------------------------
// phraseText — flatten parts to a single read-out string (aria / tests).
// --------------------------------------------------------------------------

export function phraseText(parts: PhrasePart[]): string {
  const pieces: string[] = [];
  for (const p of parts) {
    const s = p.kind === "text" ? p.text : p.display;
    if (s.length > 0) pieces.push(s);
  }
  return pieces.join(" ").replace(/\s+/g, " ").trim();
}

/** Convenience: the full read-out for a leaf ("Paloma is present"). */
export function leafPhraseText(leaf: MatchLeaf, lookup: PersonNameLookup): string {
  return phraseText(leafPhrase(leaf, lookup).parts);
}

// --------------------------------------------------------------------------
// leafSentence — at-rest natural language for the inline sentence builder
// (POSTSHIP cycle 7, per `docs/design/inline-sentence-builder.md` §4).
//
// Distinct from `leafPhrase`/`leafPhraseText` (which carry control slots for
// the old stacked PillCard): this produces the plain reading shown on a pill
// at rest and in the live readout. `location` reads as "Area N" — the number
// is assigned by document order across the sentence and passed in by the
// builder; the coordinates live in the linked numbered map block.
// --------------------------------------------------------------------------

/** "2024-07-15T00:00:00Z" → "2024-07-15"; passes through a bare date. */
function isoDateOnly(iso: string | null): string {
  if (!iso) return "";
  const m = /^(\d{4}-\d{2}-\d{2})/.exec(iso);
  return m ? m[1]! : iso;
}

export function leafSentence(
  leaf: MatchLeaf,
  lookup: PersonNameLookup,
  areaNumber?: number,
): string {
  switch (leaf.leaf) {
    case "person": {
      // Empty id means the operator hasn't picked yet — keep it reading as a
      // sentence and as a prompt to click the pill.
      const name = leaf.person_id ? personLabel(leaf.person_id, lookup) : "someone";
      switch (leaf.mode) {
        case "must_include":
          return `${name} is present`;
        case "may_include":
          return `${name} may be present`;
        case "must_exclude":
          return `${name} is not present`;
        case "includes":
          return `${name} appears`;
      }
      return name;
    }

    case "people_count":
      return `people count ${opSymbol(leaf.op)} ${leaf.value}`;

    case "face_recognition": {
      if (!leaf.allow_unrecognized) {
        return leaf.yolo_count_check
          ? "all faces must be recognized · reject extra humans (YOLO)"
          : "all faces must be recognized";
      }
      return leaf.yolo_count_check
        ? "no unidentified extra humans (YOLO)"
        : "unrecognized faces allowed";
    }

    case "date_range": {
      const from = isoDateOnly(leaf.from);
      const to = isoDateOnly(leaf.to);
      if (from && to) return `taken between ${from} and ${to}`;
      if (from) return `taken after ${from}`;
      if (to) return `taken before ${to}`;
      return "taken on any date";
    }

    case "location":
      return areaNumber !== undefined ? `taken in Area ${areaNumber}` : "taken in an area";

    case "media_type": {
      const label = mediaTypesLabel(leaf.types);
      return `is a ${label}`;
    }
  }
}
