import { describe, expect, it } from "vitest";

import { and, emptyMatch, not, or, type MatchExpr } from "../matchTree";
import {
  exceedsMaxDepth,
  getNode,
  insertChild,
  isPrefix,
  keyToPath,
  moveNode,
  normalizeTree,
  parentPath,
  pathToKey,
  removeNode,
  replaceNode,
  setGroupOp,
  toggleNot,
  wrapInGroup,
} from "../treeOps";

// --------------------------------------------------------------------------
// Leaf fixtures — distinct ids so deep-equality tells them apart.
// --------------------------------------------------------------------------

function person(id: string): MatchExpr {
  return { kind: "leaf", leaf: "person", mode: "must_include", person_id: id };
}

const A = person("aaaa");
const B = person("bbbb");
const C = person("cccc");
const D = person("dddd");

/** n nested single-child ANDs around a leaf → depth n + 1 (collapses on normalize). */
function chain(n: number): MatchExpr {
  let node: MatchExpr = A;
  for (let i = 0; i < n; i++) node = and([node]);
  return node;
}

/** n nested 2-child ANDs → depth n + 1, survives normalize (no single-child unwrap). */
function deepBranch(n: number): MatchExpr {
  let node: MatchExpr = B;
  for (let i = 0; i < n; i++) node = and([A, node]);
  return node;
}

// --------------------------------------------------------------------------

describe("treeOps — path helpers", () => {
  it("parentPath drops the last index", () => {
    expect(parentPath([0, 1, 2])).toEqual([0, 1]);
    expect(parentPath([])).toEqual([]);
  });

  it("isPrefix recognizes ancestor-or-self", () => {
    expect(isPrefix([], [0, 1])).toBe(true);
    expect(isPrefix([0], [0, 1])).toBe(true);
    expect(isPrefix([0, 1], [0, 1])).toBe(true);
    expect(isPrefix([1], [0, 1])).toBe(false);
    expect(isPrefix([0, 1], [0])).toBe(false);
  });

  it("pathToKey / keyToPath round-trip", () => {
    expect(pathToKey([])).toBe("");
    expect(keyToPath("")).toEqual([]);
    expect(pathToKey([0, 2, 1])).toBe("0.2.1");
    expect(keyToPath("0.2.1")).toEqual([0, 2, 1]);
  });

  it("getNode walks AND/OR children and the NOT child as index 0", () => {
    const root = and([A, or([B, C]), not(D)]);
    expect(getNode(root, [])).toBe(root);
    expect(getNode(root, [0])).toBe(A);
    expect(getNode(root, [1, 1])).toBe(C);
    expect(getNode(root, [2, 0])).toBe(D);
  });

  it("getNode returns null for out-of-range / through-a-leaf / bad NOT index", () => {
    const root = and([A, not(B)]);
    expect(getNode(root, [5])).toBeNull();
    expect(getNode(root, [0, 0])).toBeNull(); // A is a leaf
    expect(getNode(root, [1, 1])).toBeNull(); // NOT only has index 0
  });
});

describe("treeOps — replaceNode", () => {
  it("replaces the root when path is empty", () => {
    expect(replaceNode(and([A]), [], B)).toBe(B);
  });

  it("replaces a nested child, sharing untouched siblings", () => {
    const root = and([A, or([B, C])]);
    const next = replaceNode(root, [1, 0], D);
    expect(next).toEqual(and([A, or([D, C])]));
  });

  it("replaces a NOT's child", () => {
    const root = and([not(A)]);
    expect(replaceNode(root, [0, 0], B)).toEqual(and([not(B)]));
  });

  it("rejects a replacement that busts the depth cap", () => {
    const root = and([A, B]);
    const out = replaceNode(root, [0], deepBranch(8)); // would be depth 10
    expect(out).toBe(root);
  });
});

describe("treeOps — removeNode", () => {
  it("splices a child out of an AND group", () => {
    expect(removeNode(and([A, B, C]), [1])).toEqual(and([A, C]));
  });

  it("removing the only child of a NOT drops the NOT node", () => {
    const root = and([A, not(B)]);
    expect(removeNode(root, [1, 0])).toEqual(and([A]));
  });

  it("removing the root yields the empty state", () => {
    expect(removeNode(and([A]), [])).toEqual(emptyMatch());
  });

  it("returns the tree unchanged for an out-of-range index", () => {
    const root = and([A]);
    expect(removeNode(root, [9])).toBe(root);
  });
});

describe("treeOps — insertChild", () => {
  it("inserts at an index into an AND/OR group", () => {
    expect(insertChild(and([A, C]), [], 1, B)).toEqual(and([A, B, C]));
  });

  it("clamps an out-of-range index to append", () => {
    expect(insertChild(and([A]), [], 9, B)).toEqual(and([A, B]));
  });

  it("rejects inserting into a NOT group or a leaf", () => {
    const root = and([not(A)]);
    expect(insertChild(root, [0], 0, B)).toBe(root);
    expect(insertChild(root, [0, 0], 0, B)).toBe(root);
  });
});

describe("treeOps — moveNode", () => {
  it("reorders within a parent, shifting the index forward", () => {
    // move A (index 0) to the end
    expect(moveNode(and([A, B, C, D]), [0], [], 4)).toEqual(and([B, C, D, A]));
  });

  it("reorders within a parent, moving backward", () => {
    expect(moveNode(and([A, B, C, D]), [3], [], 0)).toEqual(and([D, A, B, C]));
  });

  it("moves a node into a sibling group, adjusting the shifted parent path", () => {
    const root = and([A, and([B])]);
    // move A (index 0) into the sibling group (originally at index 1) at pos 0
    expect(moveNode(root, [0], [1], 0)).toEqual(and([and([A, B])]));
  });

  it("rejects moving a node into its own descendant", () => {
    const root = and([and([A])]);
    expect(moveNode(root, [0], [0], 0)).toBe(root);
    expect(moveNode(root, [0], [0, 0], 0)).toBe(root);
  });

  it("rejects moving the root", () => {
    const root = and([A]);
    expect(moveNode(root, [], [0], 0)).toBe(root);
  });

  it("rejects moving a NOT's child or moving into a NOT", () => {
    const fromNot = and([not(A), B]);
    expect(moveNode(fromNot, [0, 0], [], 2)).toBe(fromNot); // source under NOT
    expect(moveNode(fromNot, [1], [0], 0)).toBe(fromNot); // target is NOT
  });

  it("rejects a move that would bust the depth cap", () => {
    const root = and([deepBranch(7), A]); // deepBranch(7) is depth 8
    // moving A into the deepest group would create depth 9
    const deepestParent = [0, 1, 1, 1, 1, 1, 1]; // walk the 2nd child each level
    expect(getNode(root, deepestParent)).not.toBeNull();
    expect(moveNode(root, [1], deepestParent, 0)).toBe(root);
  });
});

describe("treeOps — wrapInGroup", () => {
  it("wraps contiguous siblings into a new group at the lowest index", () => {
    expect(wrapInGroup(and([A, B, C, D]), [], [1, 2], "or")).toEqual(and([A, or([B, C]), D]));
  });

  it("wraps non-contiguous siblings, preserving relative order, at the lowest index", () => {
    expect(wrapInGroup(and([A, B, C, D]), [], [0, 2], "and")).toEqual(and([and([A, C]), B, D]));
  });

  it("rejects when an index is out of range or the parent is not AND/OR", () => {
    const root = and([A, B]);
    expect(wrapInGroup(root, [], [0, 9], "and")).toBe(root);
    const notRoot = and([not(A)]);
    expect(wrapInGroup(notRoot, [0], [0], "and")).toBe(notRoot);
  });

  it("rejects a wrap that would bust the depth cap", () => {
    // deepBranch(7) is depth 8; the innermost and([A, B]) is 6 steps down the 2nd child
    const root = deepBranch(7);
    const innerPath = [1, 1, 1, 1, 1, 1];
    expect(getNode(root, innerPath)).toEqual(and([A, B]));
    // wrapping A and B adds a level → depth 9 → reject
    expect(wrapInGroup(root, innerPath, [0, 1], "or")).toBe(root);
  });
});

describe("treeOps — setGroupOp", () => {
  it("flips AND to OR and back", () => {
    expect(setGroupOp(and([A, B]), [], "or")).toEqual(or([A, B]));
    expect(setGroupOp(or([A, B]), [], "and")).toEqual(and([A, B]));
  });

  it("is a no-op when the op already matches or the node is not a group", () => {
    const root = and([A]);
    expect(setGroupOp(root, [], "and")).toBe(root);
    expect(setGroupOp(root, [0], "or")).toBe(root); // A is a leaf
  });
});

describe("treeOps — toggleNot", () => {
  it("wraps a node in NOT", () => {
    expect(toggleNot(and([A, B]), [0])).toEqual(and([not(A), B]));
  });

  it("unwraps an existing NOT", () => {
    expect(toggleNot(and([not(A), B]), [0])).toEqual(and([A, B]));
  });

  it("rejects creating NOT-of-NOT", () => {
    const root = and([not(A)]);
    expect(toggleNot(root, [0, 0])).toBe(root); // A's parent is already NOT
  });

  it("rejects a wrap that would bust the depth cap", () => {
    const root = deepBranch(7); // depth 8
    expect(toggleNot(root, [])).toBe(root); // wrapping root in NOT → depth 9
  });
});

describe("treeOps — normalizeTree", () => {
  it("unwraps a single-child AND/OR group", () => {
    expect(normalizeTree(and([A]))).toEqual(A);
    expect(normalizeTree(or([and([A])]))).toEqual(A);
  });

  it("drops a dangling not(empty) then unwraps", () => {
    expect(normalizeTree(and([A, not(emptyMatch())]))).toEqual(A);
  });

  it("drops empty child groups", () => {
    expect(normalizeTree(or([A, and([])]))).toEqual(A);
    expect(normalizeTree(and([A, B, or([])]))).toEqual(and([A, B]));
  });

  it("leaves a valid multi-child tree intact and does not truncate depth", () => {
    const tree = and([A, or([B, C])]);
    expect(normalizeTree(tree)).toEqual(tree);
    expect(normalizeTree(deepBranch(7))).toEqual(deepBranch(7)); // depth 8 preserved
  });
});

describe("treeOps — exceedsMaxDepth", () => {
  it("is false at the cap and true beyond it", () => {
    expect(exceedsMaxDepth(deepBranch(7))).toBe(false); // depth 8
    expect(exceedsMaxDepth(deepBranch(8))).toBe(true); // depth 9
    expect(exceedsMaxDepth(chain(7))).toBe(false); // depth 8
  });
});
