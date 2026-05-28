// Path-addressed, pure structural edits over a `MatchExpr` tree.
//
// This is the mutation surface behind the drag-and-drop block builder
// (POSTSHIP-T35, per `docs/design/dnd-block-builder.md` §6). Drag-reorder,
// "Group selected", remove, AND/OR flip, and NOT wrap/unwrap all reduce to the
// functions here, so the rendering layer never reaches into the tree directly.
//
// A **path** is an array of child indices from the root. A NOT group's single
// `child` is addressed as index `0`. Examples: `[]` = root, `[2]` = root's 3rd
// child, `[0,1]` = first child's 2nd child, `[3,0]` = the child of the NOT at
// root index 3.
//
// Every edit is immutable: it returns a new root (reusing untouched subtree
// references). When an edit is illegal (would exceed depth 8, move a node into
// its own descendant, address a missing slot) the function returns the SAME
// `root` reference unchanged — callers detect rejection by `result === root`.

import {
  MAX_TREE_DEPTH,
  and,
  depth,
  emptyMatch,
  isEmpty,
  not,
  or,
  type MatchExpr,
} from "./matchTree";

// --------------------------------------------------------------------------
// Path helpers.
// --------------------------------------------------------------------------

export function parentPath(path: number[]): number[] {
  return path.slice(0, -1);
}

/** True when `a` addresses an ancestor-or-self of `b`. */
export function isPrefix(a: number[], b: number[]): boolean {
  if (a.length > b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

export function pathToKey(path: number[]): string {
  return path.join(".");
}

export function keyToPath(key: string): number[] {
  if (key.length === 0) return [];
  return key.split(".").map((s) => Number.parseInt(s, 10));
}

/** Walk to the node at `path`, or `null` if the path addresses no slot. */
export function getNode(root: MatchExpr, path: number[]): MatchExpr | null {
  let cur: MatchExpr = root;
  for (const idx of path) {
    if (cur.kind === "leaf") return null;
    if (cur.op === "not") {
      if (idx !== 0) return null;
      cur = cur.child;
    } else {
      if (idx < 0 || idx >= cur.children.length) return null;
      cur = cur.children[idx]!;
    }
  }
  return cur;
}

// --------------------------------------------------------------------------
// Internal: rebuild a group from a child list, preserving its op.
// --------------------------------------------------------------------------

function rebuildGroup(op: "and" | "or", children: MatchExpr[]): MatchExpr {
  return op === "and" ? and(children) : or(children);
}

/** Depth guard: reject (return the original) when applying `next` busts the cap. */
function guardDepth(original: MatchExpr, next: MatchExpr): MatchExpr {
  return depth(next) > MAX_TREE_DEPTH ? original : next;
}

export function exceedsMaxDepth(root: MatchExpr): boolean {
  return depth(root) > MAX_TREE_DEPTH;
}

// --------------------------------------------------------------------------
// replaceNode — swap the subtree at `path` for `next`.
// --------------------------------------------------------------------------

export function replaceNode(root: MatchExpr, path: number[], next: MatchExpr): MatchExpr {
  const replaced = replaceAt(root, path, 0, next);
  if (replaced === root) return root;
  return guardDepth(root, replaced);
}

function replaceAt(node: MatchExpr, path: number[], depthIdx: number, next: MatchExpr): MatchExpr {
  if (depthIdx === path.length) return next;
  if (node.kind === "leaf") return node;
  const idx = path[depthIdx]!;
  if (node.op === "not") {
    if (idx !== 0) return node;
    return not(replaceAt(node.child, path, depthIdx + 1, next));
  }
  if (idx < 0 || idx >= node.children.length) return node;
  const children = node.children.slice();
  children[idx] = replaceAt(children[idx]!, path, depthIdx + 1, next);
  return rebuildGroup(node.op, children);
}

// --------------------------------------------------------------------------
// removeNode — splice the node out of its parent. Removing a NOT's only child
// drops the NOT node itself ("unwraps NOT"). Removing the root yields the empty
// state.
// --------------------------------------------------------------------------

export function removeNode(root: MatchExpr, path: number[]): MatchExpr {
  if (path.length === 0) return emptyMatch();
  const pPath = parentPath(path);
  const parent = getNode(root, pPath);
  if (parent === null || parent.kind === "leaf") return root;
  if (parent.op === "not") {
    // A NOT can't survive losing its only child — drop the NOT entirely.
    return removeNode(root, pPath);
  }
  const lastIdx = path[path.length - 1]!;
  if (lastIdx < 0 || lastIdx >= parent.children.length) return root;
  const children = parent.children.filter((_, i) => i !== lastIdx);
  return replaceNode(root, pPath, rebuildGroup(parent.op, children));
}

// --------------------------------------------------------------------------
// insertChild — splice `node` into an AND/OR group at `index` (clamped).
// NOT groups and leaves reject (they have no insertable child list).
// --------------------------------------------------------------------------

export function insertChild(
  root: MatchExpr,
  groupPath: number[],
  index: number,
  node: MatchExpr,
): MatchExpr {
  const group = getNode(root, groupPath);
  if (group === null || group.kind === "leaf" || group.op === "not") return root;
  const children = group.children.slice();
  const clamped = Math.max(0, Math.min(index, children.length));
  children.splice(clamped, 0, node);
  return replaceNode(root, groupPath, rebuildGroup(group.op, children));
}

// --------------------------------------------------------------------------
// moveNode — relocate the node at `from` into the AND/OR group at `toParent`
// at `toIndex`. Supports reordering within a parent and moving between AND/OR
// groups (the entire drag use-case). Rejects moving into one's own descendant,
// moving the root, and moves whose source or target slot isn't an AND/OR child.
// --------------------------------------------------------------------------

export function moveNode(
  root: MatchExpr,
  from: number[],
  toParent: number[],
  toIndex: number,
): MatchExpr {
  if (from.length === 0) return root;
  if (isPrefix(from, toParent)) return root; // into self / descendant

  const node = getNode(root, from);
  if (node === null) return root;

  const fp = parentPath(from);
  const fParent = getNode(root, fp);
  if (fParent === null || fParent.kind === "leaf" || fParent.op === "not") return root;

  const target = getNode(root, toParent);
  if (target === null || target.kind === "leaf" || target.op === "not") return root;

  const fi = from[from.length - 1]!;

  // The removal of `from` splices a slot out of `fp`; adjust the target
  // coordinates so they still point where the caller meant after the splice.
  const adjParent = toParent.slice();
  let adjIndex = toIndex;
  if (isPrefix(fp, toParent)) {
    if (toParent.length === fp.length) {
      // Same parent: only the insertion index shifts.
      if (toIndex > fi) adjIndex -= 1;
    } else if (toParent[fp.length]! > fi) {
      // Target descends through `fp` at a sibling after the removed one.
      adjParent[fp.length] = adjParent[fp.length]! - 1;
    }
  }

  const removed = removeNode(root, from);
  return insertChild(removed, adjParent, adjIndex, node);
}

// --------------------------------------------------------------------------
// wrapInGroup — replace the selected sibling children (at `childIndices` of the
// AND/OR group at `parentPath`) with a single new AND/OR group containing them
// in their original relative order, placed at the lowest selected index.
// --------------------------------------------------------------------------

export function wrapInGroup(
  root: MatchExpr,
  parentPathArg: number[],
  childIndices: number[],
  op: "and" | "or",
): MatchExpr {
  const parent = getNode(root, parentPathArg);
  if (parent === null || parent.kind === "leaf" || parent.op === "not") return root;
  const idxs = Array.from(new Set(childIndices)).sort((a, b) => a - b);
  if (idxs.length === 0) return root;
  for (const i of idxs) {
    if (i < 0 || i >= parent.children.length) return root;
  }
  const collected = idxs.map((i) => parent.children[i]!);
  const newGroup = rebuildGroup(op, collected);
  const minIdx = idxs[0]!;
  const remaining: MatchExpr[] = [];
  parent.children.forEach((c, i) => {
    if (!idxs.includes(i)) remaining.push(c);
  });
  // Every child before `minIdx` is unselected (minIdx is the smallest selected
  // index), so the new group lands at position `minIdx` in the reduced list.
  remaining.splice(minIdx, 0, newGroup);
  return replaceNode(root, parentPathArg, rebuildGroup(parent.op, remaining));
}

// --------------------------------------------------------------------------
// setGroupOp — flip an AND group to OR or vice-versa.
// --------------------------------------------------------------------------

export function setGroupOp(root: MatchExpr, path: number[], op: "and" | "or"): MatchExpr {
  const node = getNode(root, path);
  if (node === null || node.kind === "leaf" || node.op === "not") return root;
  if (node.op === op) return root;
  return replaceNode(root, path, rebuildGroup(op, node.children));
}

// --------------------------------------------------------------------------
// toggleNot — wrap the node at `path` in a NOT, or unwrap it if it already is
// one. Rejects creating NOT-of-NOT (schema forbids it; UI greys the control).
// --------------------------------------------------------------------------

export function toggleNot(root: MatchExpr, path: number[]): MatchExpr {
  const node = getNode(root, path);
  if (node === null) return root;
  if (node.kind === "group" && node.op === "not") {
    return replaceNode(root, path, node.child);
  }
  if (path.length > 0) {
    const parent = getNode(root, parentPath(path));
    if (parent && parent.kind === "group" && parent.op === "not") return root;
  }
  return replaceNode(root, path, not(node));
}

// --------------------------------------------------------------------------
// normalizeTree — coerce a transiently-edited tree into a server-acceptable
// shape at the serialization boundary. NOT eager during editing.
//   - single-child AND/OR → unwrap to that child
//   - dangling `not(empty)` and empty AND/OR children → dropped by their parent
// Depth is never silently truncated (the ops reject depth>8 upstream).
// --------------------------------------------------------------------------

export function normalizeTree(root: MatchExpr): MatchExpr {
  return normalizeNode(root);
}

function normalizeNode(node: MatchExpr): MatchExpr {
  if (node.kind === "leaf") return node;
  if (node.op === "not") {
    return not(normalizeNode(node.child));
  }
  const normalized = node.children.map(normalizeNode).filter((c) => !isDroppable(c));
  if (normalized.length === 1) return normalized[0]!;
  return rebuildGroup(node.op, normalized);
}

function isDroppable(node: MatchExpr): boolean {
  if (node.kind === "leaf") return false;
  if (node.op === "not") return isEmpty(node.child);
  return node.children.length === 0;
}
