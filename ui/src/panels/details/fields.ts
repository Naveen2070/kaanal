// The Controls tab as data: where each control lives in the config, the patch that edits it, and
// the two pure sentences ("what that works out to", "suggested fix"). Nothing here draws anything,
// so the field-to-patch mapping is tested as a table. A control the config has no field for is
// not offered (retries; error rate, which is accepted but not active yet).

import { kindRulesOf } from "../../canvas/rules";
import type { NodeDoc, NodeSummary, Op } from "../../data/api/types";

type Path = (string | number)[];

const isObject = (v: unknown): v is Record<string, unknown> =>
  v !== null && typeof v === "object" && !Array.isArray(v);

/** `set` when the field is there, `insert` when it is not (a new key under a mapping). */
function put(path: Path, present: boolean, value: unknown): Op {
  return { op: present ? "set" : "insert", path, value };
}

export interface Slots {
  value: number | null;
  /** Where it lives, for the hint under the field. */
  from: "concurrency" | "pool" | null;
}

/** The most requests the node serves at once (per instance). */
export function slotsOf(config: NodeDoc): Slots {
  const c = config.concurrency;
  if (typeof c === "number") return { value: c, from: "concurrency" };
  if (isObject(c) && typeof c.limit === "number") return { value: c.limit, from: "concurrency" };
  if (isObject(config.pool) && typeof config.pool.size === "number") {
    return { value: config.pool.size, from: "pool" };
  }
  return { value: null, from: null };
}

export function setSlots(name: string, config: NodeDoc, n: number): Op[] {
  const c = config.concurrency;
  if (typeof c === "number") return [put(["nodes", name, "concurrency"], true, n)];
  if (isObject(c)) return [put(["nodes", name, "concurrency", "limit"], "limit" in c, n)];
  if (isObject(config.pool))
    return [put(["nodes", name, "pool", "size"], "size" in config.pool, n)];
  return [put(["nodes", name, "concurrency"], false, n)];
}

/**
 * The longest queue allowed before a request is turned away; "unbounded" for a bare slot count
 * (no limit, the engine's `queue: None`); null when the config has no slots. 0 is a real value:
 * nobody waits, a request is turned away as soon as every slot is busy.
 */
export function queueOf(config: NodeDoc): number | "unbounded" | null {
  if (slotsOf(config).value === null) return null;
  const c = config.concurrency;
  if (isObject(c)) return typeof c.queue === "number" ? c.queue : 0;
  if (isObject(config.pool)) return typeof config.pool.queue === "number" ? config.pool.queue : 0;
  return "unbounded";
}

/** A queue set back to "no limit": the bare slot count. Only for `concurrency`; a pool always has a queue. */
export function clearQueueLimit(name: string, config: NodeDoc): Op[] {
  const c = config.concurrency;
  return isObject(c) && typeof c.limit === "number"
    ? [put(["nodes", name, "concurrency"], true, c.limit)]
    : [];
}

export function setQueue(name: string, config: NodeDoc, n: number): Op[] {
  const c = config.concurrency;
  if (typeof c === "number") {
    // a bare number has no room for a queue: it becomes the long form
    return [put(["nodes", name, "concurrency"], true, { limit: c, queue: n })];
  }
  if (isObject(c)) return [put(["nodes", name, "concurrency", "queue"], "queue" in c, n)];
  if (isObject(config.pool))
    return [put(["nodes", name, "pool", "queue"], "queue" in config.pool, n)];
  return [];
}

const DURATION = /^(\d+(?:\.\d+)?)(ms|s)$/;

/** A field written as a plain duration (`98ms`, `2s`) in milliseconds; null for anything else. */
export function durationMs(v: unknown): number | null {
  const m = typeof v === "string" ? DURATION.exec(v) : null;
  return m ? Number(m[1]) * (m[2] === "s" ? 1000 : 1) : null;
}

/** Milliseconds as the loader reads them: plain digits, at most microseconds, never `1e-7ms`. */
export function formatMs(ms: number): string {
  return `${Number(ms.toFixed(3))}ms`;
}

export function setDuration(name: string, config: NodeDoc, field: string, ms: number): Op {
  return put(["nodes", name, field], field in config, formatMs(ms));
}

/** The fallback for the kinds that have a `proc` field (the others price their work with `ops`, `hit` and `miss`); the server's `kinds` wins. */
const HAS_PROC = new Set(["gateway", "service", "monolith", "external-api", "auth"]);

export function hasProc(kind: string): boolean {
  // the engine's own answer when the server has given it, else the copy above
  return kindRulesOf(kind)?.service_time ?? HAS_PROC.has(kind);
}

/** Limits the loader enforces (or that make no sense past): a value outside them would make the draft invalid. */
export const LIMITS = {
  slots: { min: 1, max: 100_000 },
  instances: { min: 1, max: 64 },
  queue: { min: 0, max: 1_000_000 },
  serviceMs: { min: 0, max: 60_000 },
  timeoutMs: { min: 0.001, max: 300_000 },
  rate: { min: 0.000001, max: 1_000_000 },
} as const;

/** The traffic profile's total rate (the offered-load slider). */
export function setRate(profile: string, rps: number): Op {
  return { op: "set", path: ["traffic", profile, "rps"], value: rps };
}

/** How traffic entering a node (or over a wire) picks a copy: the config's `balance`; null when unset. */
export function balanceOf(v: unknown): "round-robin" | "random" | "weighted" | null {
  if (v === "round-robin" || v === "random") return v;
  return isObject(v) && "weighted" in v ? "weighted" : null;
}

/** The patch for a chosen balance: "" removes it (the default applies), else the name is set. */
export function setBalance(
  path: Path,
  present: boolean,
  choice: "" | "round-robin" | "random",
): Op[] {
  if (choice === "") return present ? [{ op: "remove", path }] : [];
  return [put(path, present, choice)];
}

/** What a node's slot covers: its own work only (the default), or until what it calls has answered. */
export const holdOf = (config: NodeDoc): "own" | "call" =>
  config.hold === "call" ? "call" : "own";

/** The patch for a chosen hold: `own` is the default, so it removes the field. */
export function setHold(name: string, config: NodeDoc, choice: "own" | "call"): Op[] {
  const path = ["nodes", name, "hold"];
  if (choice === "own") return "hold" in config ? [{ op: "remove", path }] : [];
  return [put(path, "hold" in config, "call")];
}

/** What "Thread per request" sets on a node with no slot limit: Tomcat's defaults (200 threads, 100 waiting). */
export const THREAD_STYLE = { slots: 200, queue: 100 };

/**
 * A server style: `own` is an event loop (the default, nothing written); `call` is a thread per
 * request, which holds its slot across its calls and, with no limit yet, gets a typical pool.
 */
export function setStyle(name: string, config: NodeDoc, choice: "own" | "call"): Op[] {
  const ops = setHold(name, config, choice);
  if (choice === "call" && slotsOf(config).value === null) {
    ops.push(
      put(["nodes", name, "concurrency"], false, {
        limit: THREAD_STYLE.slots,
        queue: THREAD_STYLE.queue,
      }),
    );
  }
  return ops;
}

export function setReplicas(name: string, config: NodeDoc, n: number): Op {
  return put(["nodes", name, "replicas"], "replicas" in config, n);
}

/** Take the node out: the node, and every edge that names it (an edge to nowhere is invalid). */
export function deleteOps(name: string, edges: { from: unknown; to: unknown }[]): Op[] {
  const ops: Op[] = [{ op: "remove", path: ["nodes", name] }];
  // highest index first, so each removal leaves the indexes of the rest alone
  for (let i = edges.length - 1; i >= 0; i--) {
    const e = edges[i];
    if (e && (String(e.from) === name || String(e.to) === name)) {
      ops.push({ op: "remove", path: ["edges", i] });
    }
  }
  return ops;
}

export interface WorksOut {
  /** Visits per second the node can take: slots x instances / service time. */
  capacityPerS: number;
  /** One minus offered load over capacity: 0.4 is 40% spare, below 0 is overloaded. */
  spare: number;
}

export function worksOutTo(
  slots: number,
  serviceMs: number,
  replicas: number,
  offeredPerS = 0,
): WorksOut {
  const capacityPerS = (slots * replicas) / (serviceMs / 1000);
  return { capacityPerS, spare: 1 - offeredPerS / capacityPerS };
}

/** A sentence from what happened to the node in the run; null when nothing is wrong. */
export function suggestedFix(node: NodeSummary | undefined): string | null {
  if (!node) return null;
  if (node.overflowed > 0) {
    return "Requests were turned away because the queue was full. Raise the queue limit or add instances.";
  }
  if (node.saturated) {
    return "Add instances for more parallel slots, or lower service time if the work itself can be made faster.";
  }
  if (node.timed_out > 0) {
    return "Calls to this node ran past their deadline. Lower the service time, add instances or raise the timeout.";
  }
  return null;
}

// ---- chaos: structured targets, never "a->b" strings (parent spec) ----

export type ChaosTarget =
  | { node: string }
  | { node: string; replica: number }
  | { edge: { from: string; to: string } };

export type ChaosAction =
  | { kind: "down"; down: boolean }
  | { kind: "slow"; ms: number }
  | { kind: "drop"; rate: number }
  | { kind: "clear" };

/** The body of `PUT /__kaanal/chaos` for an action on a target. */
export function chaosBody(target: ChaosTarget, action: ChaosAction): Record<string, unknown> {
  switch (action.kind) {
    case "down":
      return { ...target, down: action.down };
    case "slow":
      return { ...target, latency_ms: action.ms };
    case "drop":
      return { ...target, error_rate: action.rate };
    case "clear":
      return { ...target, down: false, latency_ms: 0, error_rate: 0 };
  }
}

export interface Row {
  id: "time" | "slots" | "copies";
  editable: boolean;
  /** Why a row is read-only, shown in the row. */
  why?: string;
}

/**
 * The rows the control bar shows for a node (R1.99). Time is the node's `proc`: a database or cache
 * has none (its cost comes from its flavor, and a `proc` patch would make the draft invalid), and a
 * `proc` that is a distribution (or any text that is not a plain duration) is shown but not edited.
 */
export function controlRows(kind: string, config: NodeDoc): Row[] {
  const rows: Row[] = [];
  if (hasProc(kind)) {
    const plain = !("proc" in config) || durationMs(config.proc) !== null;
    rows.push(
      plain
        ? { id: "time", editable: true }
        : { id: "time", editable: false, why: "edit in kaanal.yaml" },
    );
  }
  rows.push({ id: "slots", editable: true }, { id: "copies", editable: true });
  return rows;
}
