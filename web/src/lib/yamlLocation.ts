// Read and write the `match.location` block of a rule YAML document.
//
// Targeted regex over the canonical layout — NOT a full YAML parser. Contract:
//
//   match:
//     location:
//       center: [<lat>, <lng>]
//       radius_km: <number>
//
// Not handled:
//  - flow-style mappings (`location: {center: [...], radius_km: ...}`)
//  - comments interleaved inside the location block
//  - non-integer indents that mix with sibling keys
//  - multi-document YAML (`---` separators)
//
// `writeLocation` preserves every other key inside `match:` (date, people,
// media) when (re)writing the `location:` sub-block. If `match:` is absent,
// it appends a fresh `match:\n  location: ...` block at the end.
//
// Coordinate order is [lat, lng] to match Rust `LocationPredicate.center`
// (PRD §6) and the `<MapPicker>` Solid component.

export interface LocationBlock {
  center: [number, number];
  radiusKm: number;
}

export const DEFAULT_LOCATION: LocationBlock = {
  center: [48.8566, 2.3522],
  radiusKm: 60,
};

const LOCATION_REGEX =
  /^([ \t]*)location:[ \t]*\n[ \t]+center:[ \t]*\[[ \t]*(-?[\d.]+)[ \t]*,[ \t]*(-?[\d.]+)[ \t]*\][ \t]*\n[ \t]+radius_km:[ \t]*([\d.]+)/m;

const MATCH_HEADER_REGEX = /^match:[ \t]*\n/m;

export function readLocation(yaml: string): LocationBlock | null {
  const match = LOCATION_REGEX.exec(yaml);
  if (!match) return null;
  const lat = Number.parseFloat(match[2]);
  const lng = Number.parseFloat(match[3]);
  const radius = Number.parseFloat(match[4]);
  if (!Number.isFinite(lat) || !Number.isFinite(lng) || !Number.isFinite(radius)) {
    return null;
  }
  return { center: [lat, lng], radiusKm: radius };
}

export function writeLocation(yaml: string, loc: LocationBlock): string {
  const existing = LOCATION_REGEX.exec(yaml);
  if (existing) {
    const indent = existing[1];
    const replacement = renderLocation(indent, loc);
    return yaml.slice(0, existing.index) + replacement + yaml.slice(existing.index + existing[0].length);
  }

  const matchHeader = MATCH_HEADER_REGEX.exec(yaml);
  if (matchHeader) {
    const bodyStart = matchHeader.index + matchHeader[0].length;
    const bodyEnd = findMatchBodyEnd(yaml, bodyStart);
    const block = renderLocation("  ", loc) + "\n";
    const before = yaml.slice(0, bodyEnd);
    const after = yaml.slice(bodyEnd);
    const separator = before.length === 0 || before.endsWith("\n") ? "" : "\n";
    return before + separator + block + after;
  }

  const block = "match:\n" + renderLocation("  ", loc) + "\n";
  if (yaml.length === 0) return block;
  const separator = yaml.endsWith("\n") ? "" : "\n";
  return yaml + separator + block;
}

function renderLocation(indent: string, loc: LocationBlock): string {
  const inner = indent + "  ";
  const lat = formatCoord(loc.center[0]);
  const lng = formatCoord(loc.center[1]);
  const radius = formatRadius(loc.radiusKm);
  return (
    indent +
    "location:\n" +
    inner +
    "center: [" +
    lat +
    ", " +
    lng +
    "]\n" +
    inner +
    "radius_km: " +
    radius
  );
}

function formatCoord(n: number): string {
  if (!Number.isFinite(n)) return "0";
  return Number(n.toFixed(5)).toString();
}

function formatRadius(n: number): string {
  if (!Number.isFinite(n)) return "0";
  return Number(n.toFixed(2)).toString();
}

function findMatchBodyEnd(yaml: string, bodyStart: number): number {
  let pos = bodyStart;
  while (pos < yaml.length) {
    const lineEnd = yaml.indexOf("\n", pos);
    const lineStop = lineEnd === -1 ? yaml.length : lineEnd;
    const line = yaml.slice(pos, lineStop);
    if (line.length > 0 && !/^[ \t]/.test(line)) {
      return pos;
    }
    pos = lineEnd === -1 ? yaml.length : lineEnd + 1;
  }
  return yaml.length;
}
