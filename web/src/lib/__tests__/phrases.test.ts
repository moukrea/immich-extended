import { describe, expect, it } from "vitest";

import type {
  DateRangeLeaf,
  FaceRecognitionLeaf,
  LocationLeaf,
  MediaTypeLeaf,
  PeopleCountLeaf,
  PeopleCountOp,
  PersonLeaf,
  PersonMode,
} from "../matchTree";
import {
  formatLatLng,
  leafPhrase,
  leafPhraseText,
  mediaTypesLabel,
  OP_SYMBOL,
  opSymbol,
  personLabel,
  phraseText,
} from "../phrases";

// People lookup stub: "p1" → "Paloma", "p2" → "Emeric", everything else unknown.
const lookup = (id: string): string | undefined =>
  ({ p1: "Paloma", p2: "Emeric", p3: "Manon" })[id];

function person(mode: PersonMode, person_id: string): PersonLeaf {
  return { kind: "leaf", leaf: "person", mode, person_id };
}
function peopleCount(op: PeopleCountOp, value: number): PeopleCountLeaf {
  return { kind: "leaf", leaf: "people_count", op, value };
}
function face(allow_unrecognized: boolean, yolo_count_check: boolean): FaceRecognitionLeaf {
  return { kind: "leaf", leaf: "face_recognition", allow_unrecognized, yolo_count_check };
}
function dateRange(from: string | null, to: string | null): DateRangeLeaf {
  return { kind: "leaf", leaf: "date_range", from, to };
}
function location(center: [number, number], radius_km: number): LocationLeaf {
  return { kind: "leaf", leaf: "location", center, radius_km };
}
function media(types: ("photo" | "video")[]): MediaTypeLeaf {
  return { kind: "leaf", leaf: "media_type", types };
}

describe("phrases — operator symbols", () => {
  it("maps every op to its mathematical symbol", () => {
    expect(opSymbol("eq")).toBe("=");
    expect(opSymbol("ne")).toBe("≠");
    expect(opSymbol("lt")).toBe("<");
    expect(opSymbol("lte")).toBe("≤");
    expect(opSymbol("gt")).toBe(">");
    expect(opSymbol("gte")).toBe("≥");
  });

  it("OP_SYMBOL covers all six ops", () => {
    expect(Object.keys(OP_SYMBOL).sort()).toEqual(["eq", "gt", "gte", "lt", "lte", "ne"]);
  });
});

describe("phrases — person", () => {
  it("must_include reads 'X is present'", () => {
    expect(leafPhraseText(person("must_include", "p1"), lookup)).toBe("Paloma is present");
  });

  it("may_include reads 'X may be present'", () => {
    expect(leafPhraseText(person("may_include", "p1"), lookup)).toBe("Paloma may be present");
  });

  it("includes reads 'X appears'", () => {
    expect(leafPhraseText(person("includes", "p2"), lookup)).toBe("Emeric appears");
  });

  it("must_exclude reads 'never X' with the blacklist icon", () => {
    const phrase = leafPhrase(person("must_exclude", "p3"), lookup);
    expect(phrase.icon).toBe("🚫");
    expect(phraseText(phrase.parts)).toBe("never Manon");
  });

  it("the include modes use the person icon", () => {
    expect(leafPhrase(person("must_include", "p1"), lookup).icon).toBe("👤");
    expect(leafPhrase(person("may_include", "p1"), lookup).icon).toBe("👤");
    expect(leafPhrase(person("includes", "p1"), lookup).icon).toBe("👤");
  });

  it("the name is a control slot so the pill can mount a picker", () => {
    const parts = leafPhrase(person("must_include", "p1"), lookup).parts;
    expect(parts[0]).toEqual({ kind: "control", control: "person", display: "Paloma" });
  });

  it("falls back to a short id when the name is unknown", () => {
    expect(personLabel("0123456789abcdef", lookup)).toBe("01234567");
    expect(leafPhraseText(person("must_include", "0123456789abcdef"), lookup)).toBe(
      "01234567 is present",
    );
  });
});

describe("phrases — people count", () => {
  it("reads 'people count <op> <value>'", () => {
    expect(leafPhraseText(peopleCount("eq", 1), lookup)).toBe("people count = 1");
    expect(leafPhraseText(peopleCount("gte", 2), lookup)).toBe("people count ≥ 2");
    expect(leafPhraseText(peopleCount("ne", 0), lookup)).toBe("people count ≠ 0");
  });

  it("op and value are separate control slots", () => {
    const parts = leafPhrase(peopleCount("gte", 2), lookup).parts;
    expect(parts).toEqual([
      { kind: "text", text: "people count" },
      { kind: "control", control: "people_count_op", display: "≥" },
      { kind: "control", control: "people_count_value", display: "2" },
    ]);
  });
});

describe("phrases — face recognition (both phrasings)", () => {
  it("allow_unrecognized=false → 'all faces must be recognized'", () => {
    expect(leafPhraseText(face(false, false), lookup)).toBe("all faces must be recognized");
  });

  it("allow_unrecognized=false + yolo → adds 'also reject extra humans (YOLO)'", () => {
    expect(leafPhraseText(face(false, true), lookup)).toBe(
      "all faces must be recognized · also reject extra humans (YOLO)",
    );
  });

  it("allow_unrecognized=true + yolo → 'no unidentified extra humans (YOLO)'", () => {
    expect(leafPhraseText(face(true, true), lookup)).toBe("no unidentified extra humans (YOLO)");
  });

  it("allow_unrecognized=true, no yolo → 'unrecognized faces allowed'", () => {
    expect(leafPhraseText(face(true, false), lookup)).toBe("unrecognized faces allowed");
  });

  it("carries no control slots (PillCard renders its own checkboxes)", () => {
    const parts = leafPhrase(face(false, true), lookup).parts;
    expect(parts.every((p) => p.kind === "text")).toBe(true);
  });
});

describe("phrases — date range (absent bounds)", () => {
  it("both bounds → 'taken from X to Y'", () => {
    expect(leafPhraseText(dateRange("2024-07-15", "2024-07-22"), lookup)).toBe(
      "taken from 2024-07-15 to 2024-07-22",
    );
  });

  it("only a from bound → 'taken after X'", () => {
    expect(leafPhraseText(dateRange("2024-07-15", null), lookup)).toBe("taken after 2024-07-15");
  });

  it("only a to bound → 'taken before Y'", () => {
    expect(leafPhraseText(dateRange(null, "2024-07-22"), lookup)).toBe("taken before 2024-07-22");
  });

  it("no bounds → 'taken on any date'", () => {
    expect(leafPhraseText(dateRange(null, null), lookup)).toBe("taken on any date");
  });
});

describe("phrases — location", () => {
  it("reads 'within <km> km of (lat, lng)'", () => {
    expect(leafPhraseText(location([48.857, 2.352], 60), lookup)).toBe(
      "within 60 km of (48.857, 2.352)",
    );
  });

  it("trims coordinates to at most 4 decimals without trailing zeros", () => {
    expect(formatLatLng([48.8566, 2.3522])).toBe("(48.8566, 2.3522)");
    expect(formatLatLng([48.85, 2.3])).toBe("(48.85, 2.3)");
    expect(formatLatLng([48.857123456, 2.3])).toBe("(48.8571, 2.3)");
  });

  it("radius is an editable control slot", () => {
    const parts = leafPhrase(location([1, 2], 5), lookup).parts;
    expect(parts[1]).toEqual({ kind: "control", control: "location_radius", display: "5" });
  });
});

describe("phrases — media type (both → 'photo or video')", () => {
  it("single types", () => {
    expect(leafPhraseText(media(["photo"]), lookup)).toBe("is a photo");
    expect(leafPhraseText(media(["video"]), lookup)).toBe("is a video");
  });

  it("both types read 'is a photo or video' regardless of order", () => {
    expect(leafPhraseText(media(["photo", "video"]), lookup)).toBe("is a photo or video");
    expect(leafPhraseText(media(["video", "photo"]), lookup)).toBe("is a photo or video");
  });

  it("mediaTypesLabel helper", () => {
    expect(mediaTypesLabel(["photo"])).toBe("photo");
    expect(mediaTypesLabel(["video"])).toBe("video");
    expect(mediaTypesLabel(["photo", "video"])).toBe("photo or video");
  });

  it("uses the film icon", () => {
    expect(leafPhrase(media(["photo"]), lookup).icon).toBe("🎞");
  });
});

describe("phrases — phraseText", () => {
  it("skips empty control displays and collapses whitespace", () => {
    expect(
      phraseText([
        { kind: "text", text: "taken after" },
        { kind: "control", control: "date_from", display: "2024-07-15" },
        { kind: "control", control: "date_to", display: "" },
      ]),
    ).toBe("taken after 2024-07-15");
  });
});
