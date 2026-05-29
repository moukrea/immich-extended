// SentenceModel — the inline natural-language rule builder's source of truth
// (POSTSHIP cycle 7, per `docs/design/inline-sentence-builder.md` §3/§6).
//
// The builder edits a flat "sentence": a primary clause ("Include to album if
// …") plus zero or more "except if" clauses, each clause being an ordered
// all(AND)/any(OR) list of leaf conditions. This module is the *pure* mapping
// between that SentenceModel and the server's `MatchExpr` tree — no rendering,
// independently unit-tested.
//
//   props.expr ──treeToSentence──▶ SentenceModel ──(edit)──▶ sentenceToTree ──▶ onChange
//
// `treeToSentence` is conservative: trees that don't fit the flat sentence
// shape (Or-of-Ands, nested groups, `Person{includes}`, double-NOT) return
// `null`, which the builder surfaces as the Advanced-YAML fallback. It NEVER
// silently rewrites a non-fitting tree (cycle-7 ABSOLUTE rule).

import {
  and,
  isEmpty,
  not,
  or,
  type LocationLeaf,
  type MatchExpr,
  type MatchLeaf,
  type NotGroup,
} from "./matchTree";
import { formatLatLng, leafSentence, type PersonNameLookup } from "./phrases";

export type Fill = "include" | "exclude";
export type ClauseMode = "all" | "any";

export interface Clause {
  mode: ClauseMode;
  pills: MatchLeaf[];
}

export interface SentenceModel {
  fill: Fill;
  primary: Clause;
  excepts: Clause[];
}

// --------------------------------------------------------------------------
// Construction helpers.
// --------------------------------------------------------------------------

export function emptySentence(): SentenceModel {
  return { fill: "include", primary: { mode: "all", pills: [] }, excepts: [] };
}

// --------------------------------------------------------------------------
// Geo areas (L3) — a derived view. A `Location` leaf renders inline as "taken
// in Area N", numbered by document order, and edited in a numbered `MapPicker`
// block below the sentence. The number and the linked block are computed from
// the model each render — there is no stored `areas` field.
// --------------------------------------------------------------------------

/** A stable handle to a `Location` pill so its map block can edit that leaf. */
export type AreaRef =
  | { clause: "primary"; pill: number }
  | { clause: "except"; except: number; pill: number };

export interface AreaEntry {
  ref: AreaRef;
  leaf: LocationLeaf;
}

/**
 * Every `Location` leaf with its position, in document order: the primary
 * clause first, then each except clause. The index in the returned array (+1)
 * is the displayed "Area N" — so adding/removing a location renumbers the rest.
 */
export function locationAreas(model: SentenceModel): AreaEntry[] {
  const out: AreaEntry[] = [];
  model.primary.pills.forEach((pill, pillIndex) => {
    if (pill.leaf === "location") {
      out.push({ ref: { clause: "primary", pill: pillIndex }, leaf: pill });
    }
  });
  model.excepts.forEach((clause, exceptIndex) => {
    clause.pills.forEach((pill, pillIndex) => {
      if (pill.leaf === "location") {
        out.push({ ref: { clause: "except", except: exceptIndex, pill: pillIndex }, leaf: pill });
      }
    });
  });
  return out;
}

// --------------------------------------------------------------------------
// sentence → tree (§6.1).
// --------------------------------------------------------------------------

function clauseExpr(clause: Clause): MatchExpr {
  const leaves = clause.pills;
  // A single pill MUST serialize as a bare leaf — `And[leaf]`/`Or[leaf]` trips
  // the validator's "≥2 children" rule (`redundant_group`).
  if (leaves.length === 1) return leaves[0]!;
  return clause.mode === "all" ? and(leaves) : or(leaves);
}

function baseMatch(model: SentenceModel): MatchExpr {
  const primary = clauseExpr(model.primary);
  // An empty primary can't anchor any "except" — without a base condition
  // there is nothing to subtract from. Emit just the (empty) primary so the
  // tree never degenerates into `Not(And[])`/double-NOT.
  if (model.primary.pills.length === 0) return primary;
  // Drop "except if" clauses that have no conditions yet (the operator just
  // clicked "+ Except clause"). An empty clause serializes as `Not(And[])`,
  // which `normalizeTree` strips anyway — filtering here keeps `sentenceToTree`
  // self-consistent so the echo-guard never re-seeds and clears the open clause.
  const exceptNots = model.excepts
    .filter((c) => c.pills.length > 0)
    .map((c) => not(clauseExpr(c)));
  if (exceptNots.length === 0) return primary;
  return and([primary, ...exceptNots]);
}

export function sentenceToTree(model: SentenceModel): MatchExpr {
  const base = baseMatch(model);
  // `base` is always a leaf/And/Or — never a Not — so `Not(base)` is a single
  // level and can't form the forbidden `Not(Not(...))`.
  return model.fill === "include" ? base : not(base);
}

// --------------------------------------------------------------------------
// tree → sentence (§6.2) — the conservative loader.
// --------------------------------------------------------------------------

/** A leaf the builder can render as a pill: anything except `Person{includes}`. */
function isPillLeaf(expr: MatchExpr): expr is MatchLeaf {
  if (expr.kind !== "leaf") return false;
  if (expr.leaf === "person" && expr.mode === "includes") return false;
  return true;
}

function isNot(expr: MatchExpr): expr is NotGroup {
  return expr.kind === "group" && expr.op === "not";
}

interface Split {
  primary: Clause;
  excepts: Clause[];
}

/** Build a clause from the child of an except's `Not(...)`. */
function clauseFromExpr(expr: MatchExpr): Clause | null {
  if (isPillLeaf(expr)) return { mode: "all", pills: [expr] };
  if (expr.kind === "group" && expr.op === "and" && expr.children.every(isPillLeaf)) {
    return { mode: "all", pills: expr.children as MatchLeaf[] };
  }
  if (expr.kind === "group" && expr.op === "or" && expr.children.every(isPillLeaf)) {
    return { mode: "any", pills: expr.children as MatchLeaf[] };
  }
  return null;
}

function splitBase(base: MatchExpr): Split | null {
  if (isPillLeaf(base)) {
    return { primary: { mode: "all", pills: [base] }, excepts: [] };
  }
  if (base.kind === "group" && base.op === "or") {
    if (base.children.every(isPillLeaf)) {
      return { primary: { mode: "any", pills: base.children as MatchLeaf[] }, excepts: [] };
    }
    return null;
  }
  if (base.kind === "group" && base.op === "and") {
    const nots = base.children.filter(isNot);
    const nonNots = base.children.filter((c) => !isNot(c));
    const excepts: Clause[] = [];
    for (const n of nots) {
      const clause = clauseFromExpr(n.child);
      if (clause === null) return null;
      excepts.push(clause);
    }
    if (nonNots.length === 0) return null;
    let primary: Clause;
    const lone = nonNots[0]!;
    if (nonNots.length === 1 && lone.kind === "group" && lone.op === "or" && lone.children.every(isPillLeaf)) {
      primary = { mode: "any", pills: lone.children as MatchLeaf[] };
    } else if (nonNots.every(isPillLeaf)) {
      primary = { mode: "all", pills: nonNots as MatchLeaf[] };
    } else {
      return null;
    }
    return { primary, excepts };
  }
  return null;
}

export function treeToSentence(expr: MatchExpr): SentenceModel | null {
  // A brand-new / emptied rule is a valid empty "include" sentence, not a
  // fallback — never strand the operator on the YAML panel for an empty match.
  if (isEmpty(expr)) return emptySentence();

  let fill: Fill = "include";
  let base = expr;
  if (isNot(expr)) {
    fill = "exclude";
    base = expr.child;
  }
  if (isNot(base)) return null; // double-NOT ⇒ fallback

  const split = splitBase(base);
  if (split === null) return null;
  return { fill, primary: split.primary, excepts: split.excepts };
}

// --------------------------------------------------------------------------
// Live readout (§4.1) — pure, so it is unit-testable and reused by ReadoutLine.
// --------------------------------------------------------------------------

export function sentenceReadout(model: SentenceModel, lookup: PersonNameLookup): string {
  let areaCounter = 0;
  const areaLegend: string[] = [];

  const renderClause = (clause: Clause): string => {
    const connector = clause.mode === "all" ? " and " : " or ";
    const parts = clause.pills.map((leaf) => {
      if (leaf.leaf === "location") {
        areaCounter += 1;
        areaLegend.push(`${areaCounter} = within ${leaf.radius_km} km of ${formatLatLng(leaf.center)}`);
        return leafSentence(leaf, lookup, areaCounter);
      }
      return leafSentence(leaf, lookup);
    });
    return parts.join(connector);
  };

  const lead = model.fill === "include" ? "Include to album if" : "Exclude from album if";
  const primary = renderClause(model.primary);
  let sentence = primary.length > 0 ? `${lead} ${primary}.` : `${lead} …`;
  for (const clause of model.excepts) {
    const text = renderClause(clause);
    if (text.length > 0) sentence += ` Except if ${text}.`;
  }
  if (areaLegend.length > 0) sentence += ` Areas: ${areaLegend.join("; ")}.`;
  return sentence;
}
