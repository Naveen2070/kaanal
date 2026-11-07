// The step editor's model (Part 9, S1): a route's `chain:` read into steps, and the patches that edit
// it. Pure, so it is tested without a panel. Every patch is a handful of ordinary ops (one undo step),
// and the editor never writes a key it did not read: a `call:` step's `with`, `via` and inline `chain`
// stay exactly where they are, and a step in a shape it does not know is a locked row.
//
// The draft API has `set`, `insert` (a new key), `append`, `remove` and `swap` (two items of one list
// trade places as text, so each keeps its own comments). A move is one `swap` of two neighbours.

import type { Doc, Op } from "../../data/api/types";

export type Path = (string | number)[];

/** The step fields a row can edit; anything else on a step is kept as written. */
export const CALL_FIELDS = ["op", "repeat", "prob", "sends", "returns", "stream", "retry"] as const;
export type CallField = (typeof CALL_FIELDS)[number];
const EDITED = new Set<string>(["call", ...CALL_FIELDS]);

interface Base {
  /** Where the step sits in the document. */
  path: Path;
  /** The step as written, kept so a move can write it back untouched. */
  raw: unknown;
}

export interface CallStep extends Base {
  kind: "call";
  node: string;
  /** Written as a bare node name (`- db`). */
  bare: boolean;
  op?: string;
  repeat?: number;
  prob?: number;
  sends?: unknown;
  returns?: unknown;
  stream?: unknown;
  retry?: unknown;
  /** Keys on the step that the editor does not edit (`with`, `via`, an inline `chain`, ...). */
  kept: string[];
}

export interface CacheStep extends Base {
  kind: "cache";
  node: string;
}

export interface Branch {
  /** Written as a list (`[a, b]`) rather than one bare step. */
  list: boolean;
  steps: Step[];
  /** Where the branch sits in the document, and as written. */
  path: Path;
  raw: unknown;
}

export interface ParallelStep extends Base {
  kind: "parallel";
  branches: Branch[];
}

/** A step in a shape the editor does not model; it is shown as it is and never rewritten. */
export interface LockedStep extends Base {
  kind: "locked";
}

export type Step = CallStep | CacheStep | ParallelStep | LockedStep;

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

function readStep(raw: unknown, path: Path): Step {
  // the loader reads a name, an integer or a bool as a node name (`- 1`), never a float
  if (
    typeof raw === "string" ||
    typeof raw === "boolean" ||
    (typeof raw === "number" && Number.isInteger(raw))
  ) {
    return { kind: "call", path, raw, node: String(raw), bare: true, kept: [] };
  }
  if (!isObj(raw)) return { kind: "locked", path, raw };
  if (typeof raw.cache === "string" && Object.keys(raw).length === 1) {
    return { kind: "cache", path, raw, node: raw.cache };
  }
  if (Array.isArray(raw.parallel) && Object.keys(raw).length === 1) {
    const branches = raw.parallel.map((b, i): Branch => {
      const at = [...path, "parallel", i];
      return Array.isArray(b)
        ? { list: true, steps: b.map((s, j) => readStep(s, [...at, j])), path: at, raw: b }
        : { list: false, steps: [readStep(b, at)], path: at, raw: b };
    });
    return { kind: "parallel", path, raw, branches };
  }
  if (typeof raw.call === "string") {
    const step: CallStep = {
      kind: "call",
      path,
      raw,
      node: raw.call,
      bare: false,
      kept: Object.keys(raw).filter((k) => !EDITED.has(k)),
    };
    if (typeof raw.op === "string") step.op = raw.op;
    if (typeof raw.repeat === "number") step.repeat = raw.repeat;
    if (typeof raw.prob === "number") step.prob = raw.prob;
    for (const k of ["sends", "returns", "stream", "retry"] as const) {
      if (k in raw) step[k] = raw[k];
    }
    return step;
  }
  return { kind: "locked", path, raw };
}

/** The steps of a chain written as a list at `base`; `null` when it is not a list. */
export function readChain(chain: unknown, base: Path): Step[] | null {
  return Array.isArray(chain) ? chain.map((s, i) => readStep(s, [...base, i])) : null;
}

/** The route's own `chain:` as written, or `undefined` when it has none (the default chain runs). */
export function chainOf(doc: Doc | null | undefined, key: string): unknown {
  const eps = doc?.endpoints;
  const entry = isObj(eps) ? eps[key] : undefined;
  return isObj(entry) ? entry.chain : undefined;
}

/** The patch that sets one field of a step; `undefined` clears it. A bare name becomes a map first. */
export function setFieldOps(step: CallStep, field: CallField, value: unknown): Op[] {
  if (step.bare) {
    return value === undefined
      ? []
      : [{ op: "set", path: step.path, value: { call: step.node, [field]: value } }];
  }
  const path = [...step.path, field];
  const has = isObj(step.raw) && field in step.raw;
  if (value === undefined) return has ? [{ op: "remove", path }] : [];
  return [{ op: has ? "set" : "insert", path, value }];
}

/** The patch that points a call step at another component. */
export const setCalleeOps = (step: CallStep, node: string): Op[] =>
  node === step.node
    ? []
    : [
        step.bare
          ? { op: "set", path: step.path, value: node }
          : { op: "set", path: [...step.path, "call"], value: node },
      ];

/** The patch that points a cache step at another cache node. */
export const setCacheOps = (step: CacheStep, node: string): Op[] =>
  node === step.node ? [] : [{ op: "set", path: [...step.path, "cache"], value: node }];

/** The patch that adds a step at the end of the route's chain, creating the chain (and the route entry) if need be. */
export function addStepOps(doc: Doc, key: string, step: unknown): Op[] {
  const eps = doc.endpoints;
  if (!isObj(eps)) {
    return [{ op: "insert", path: ["endpoints"], value: { [key]: { chain: [step] } } }];
  }
  const entry = eps[key];
  if (!isObj(entry)) {
    return [{ op: "insert", path: ["endpoints", key], value: { chain: [step] } }];
  }
  const path = ["endpoints", key, "chain"];
  return Array.isArray(entry.chain)
    ? [{ op: "append", path, value: step }]
    : [{ op: "insert", path, value: [step] }];
}

/** The patch that adds a step to the end of a branch; a branch written as one bare step becomes a list first. */
export const addToBranchOps = (b: Branch, step: unknown): Op[] =>
  b.list
    ? [{ op: "append", path: b.path, value: step }]
    : [{ op: "set", path: b.path, value: [b.raw, step] }];

/** The patch that adds a branch to a parallel block. */
export const addBranchOps = (p: ParallelStep, step: unknown): Op[] => [
  { op: "append", path: [...p.path, "parallel"], value: step },
];

/** The patch that deletes a branch; the last one stays (delete the block instead). */
export const removeBranchOps = (p: ParallelStep, i: number): Op[] => {
  const b = p.branches[i];
  return b && p.branches.length > 1 ? [{ op: "remove", path: b.path }] : [];
};

/** The patch that deletes a step. */
export const removeStepOps = (step: Step): Op[] => [{ op: "remove", path: step.path }];

/** The patch that swaps the step at `i` with its neighbour (`dir` -1 up, +1 down), one text swap; nothing at an end. */
export function moveOps(steps: readonly Step[], i: number, dir: -1 | 1): Op[] {
  const a = steps[i];
  const b = steps[i + dir];
  if (!a || !b) return [];
  return [{ op: "swap", path: a.path, with: b.path }];
}
