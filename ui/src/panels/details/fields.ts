// The Controls tab as data: where each control lives in the config, the patch that edits it, and
// the two pure sentences ("what that works out to", "suggested fix"). Nothing here draws anything,
// so the field-to-patch mapping is tested as a table. A control the config has no field for is
// not offered (retries; error rate, which is accepted but not active yet).

import { kindRulesOf } from "../../canvas/rules";
import type { NodeDoc, NodeSummary, Op } from "../../data/api/types";
import { parseBytes } from "./units";

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
const HAS_PROC = new Set([
  "gateway",
  "service",
  "monolith",
  "external-api",
  "auth",
  "load-balancer",
]);

export function hasProc(kind: string): boolean {
  // the engine's own answer when the server has given it, else the copy above
  return kindRulesOf(kind)?.service_time ?? HAS_PROC.has(kind);
}

// ---- storage: `first_byte` is its time (a storage node refuses `proc`), `throughput` its transfer rate ----

/** A storage node's `first_byte` as plain milliseconds; null when unset or a spread of values. */
export const firstByteMs = (config: NodeDoc): number | null => durationMs(config.first_byte);

/** `first_byte` is a plain duration (one box) or absent; a `{ p50, p99 }` spread has its own two boxes, anything else (`{ mean }`) stays in the file. */
export const firstByteEditable = (config: NodeDoc): boolean =>
  !("first_byte" in config) || firstByteMs(config) !== null;

/** The two ends of a `{ p50, p99 }` first byte in milliseconds; null when it is not that shape. */
export function firstByteSpread(config: NodeDoc): { p50: number; p99: number } | null {
  const f = config.first_byte;
  if (!isObject(f)) return null;
  const p50 = durationMs(f.p50);
  const p99 = durationMs(f.p99);
  return p50 === null || p99 === null ? null : { p50, p99 };
}

/** A `{ mean }` first byte (exponential) in milliseconds; null when it is not that shape. */
export function firstByteMean(config: NodeDoc): number | null {
  const f = config.first_byte;
  return isObject(f) && Object.keys(f).length === 1 ? durationMs(f.mean) : null;
}

export const setFirstByteMean = (name: string, ms: number): Op[] => [
  put(["nodes", name, "first_byte"], true, { mean: formatMs(ms) }),
];

/** One end of the spread changed: the whole pair is written at once so p50 never sits above a stale p99 in between. */
export function setSpread(name: string, config: NodeDoc, end: "p50" | "p99", ms: number): Op[] {
  const cur = firstByteSpread(config);
  if (cur === null) return [];
  const next = { ...cur, [end]: ms };
  return [
    put(["nodes", name, "first_byte"], true, { p50: formatMs(next.p50), p99: formatMs(next.p99) }),
  ];
}

/** A node's `rate_limit` (`rps`, `burst`); null when it is absent or not that shape. */
export function rateLimitOf(config: NodeDoc): { rps: number; burst: number } | null {
  const r = config.rate_limit;
  return isObject(r) && typeof r.rps === "number" && typeof r.burst === "number"
    ? { rps: r.rps, burst: r.burst }
    : null;
}

/** The limit "Add" starts with: S3's published 5,500 GET/HEAD requests a second per prefix (GCS and Azure publish similar orders of magnitude); the burst is the same figure. */
export const DEFAULT_RATE_LIMIT = { rps: 5500, burst: 5500 } as const;

/** The limit "Add" starts with on any other kind: no provider publishes a figure for a service or a database, so it is a labelled placeholder, not an estimate of anything. */
export const GENERIC_RATE_LIMIT = { rps: 1000, burst: 1000 } as const;

/** Adds a starting `rate_limit` when the node has none: S3's figure for a store, a placeholder for the rest. */
export const addRateLimit = (name: string, config: NodeDoc): Op[] =>
  "rate_limit" in config
    ? []
    : [
        put(
          ["nodes", name, "rate_limit"],
          false,
          config.type === "storage" ? DEFAULT_RATE_LIMIT : GENERIC_RATE_LIMIT,
        ),
      ];

/** One field of the rate limit changed (the map is rewritten whole); null removes the limit. */
export function setRateLimit(
  name: string,
  config: NodeDoc,
  field: "rps" | "burst" | null,
  n = 0,
): Op[] {
  const path = ["nodes", name, "rate_limit"];
  if (field === null) return "rate_limit" in config ? [{ op: "remove", path }] : [];
  const cur = rateLimitOf(config);
  if (cur === null) return [];
  return [put(path, true, { ...cur, [field]: field === "burst" ? Math.max(1, Math.round(n)) : n })];
}

/** A storage node's throughput in KB/s (1024 bytes); null = no limit on the size cost. */
export function throughputKb(config: NodeDoc): number | null {
  const bytes = parseBytes(config.throughput, true);
  return bytes === null ? null : Number((bytes / 1024).toFixed(3));
}

/** The patch for a throughput of `kb` KB/s: whole bytes a second (at least 1); null removes the field. */
export function setThroughput(name: string, config: NodeDoc, kb: number | null): Op[] {
  const path = ["nodes", name, "throughput"];
  const present = "throughput" in config;
  if (kb === null) return present ? [{ op: "remove", path }] : [];
  return [put(path, present, Math.max(1, Math.round(kb * 1024)))];
}

export const FLAVORS = ["s3", "gcs", "blob"] as const;

export function setFlavor(name: string, config: NodeDoc, flavor: string): Op {
  return put(["nodes", name, "flavor"], "flavor" in config, flavor);
}

/** The starter values of a new storage node (`kind_defaults.storage`): estimates, not published figures. */
export function carriesEstimates(config: NodeDoc): boolean {
  return (
    durationMs(config.first_byte) === 25 && parseBytes(config.throughput, true) === 80 * 1024 * 1024
  );
}

/** Limits the loader enforces (or that make no sense past): a value outside them would make the draft invalid. */
export const LIMITS = {
  slots: { min: 1, max: 100_000 },
  instances: { min: 1, max: 64 },
  queue: { min: 0, max: 1_000_000 },
  serviceMs: { min: 0, max: 60_000 },
  timeoutMs: { min: 0.001, max: 300_000 },
  cores: { min: 0.001, max: 4096 },
  rate: { min: 0.000001, max: 1_000_000 },
} as const;

/** The traffic profile's total rate (the offered-load slider). */
export function setRate(profile: string, rps: number): Op {
  return { op: "set", path: ["traffic", profile, "rps"], value: rps };
}

/** A balance the page can write: `least-conn` and `p2c` choose by the live queues, so only the simulator can. */
export type BalanceChoice = "round-robin" | "random" | "least-conn" | "p2c";

/** How traffic entering a node (or over a wire) picks a copy: the config's `balance`; null when unset. */
export function balanceOf(v: unknown): BalanceChoice | "weighted" | null {
  if (v === "round-robin" || v === "random" || v === "least-conn" || v === "p2c") return v;
  return isObject(v) && "weighted" in v ? "weighted" : null;
}

/** The patch for a chosen balance: "" removes it (the default applies), else the name is set. */
export function setBalance(path: Path, present: boolean, choice: "" | BalanceChoice): Op[] {
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
  if (node.short_circuited > 0) {
    return "A caller's circuit breaker opened on this node and refused calls. Find out why it was failing or slow before the breaker opened.";
  }
  return null;
}

// ---- chaos: structured targets, never "a->b" strings (parent spec) ----

export type ChaosTarget =
  | { node: string }
  | { node: string; replica: number }
  | { edge: { from: string; to: string } };

export type ChaosAction =
  /** `fromMs`: milliseconds into the run the outage begins (a node or instance only); absent = from the start. */
  | { kind: "down"; down: boolean; fromMs?: number }
  | { kind: "slow"; ms: number }
  | { kind: "drop"; rate: number }
  | { kind: "clear" };

/** The longest start the box takes: a day into the run. */
const MAX_START_S = 86_400;

/**
 * The "Starting at" box, in seconds, as the milliseconds the file holds: `undefined` when it is empty
 * (the outage begins with the run), `null` when it is not a value the loader would take (nothing is
 * sent then).
 */
export function startMsOf(text: string): number | undefined | null {
  const t = text.trim();
  if (t === "") return undefined;
  const s = Number(t);
  return Number.isFinite(s) && s >= 0 && s <= MAX_START_S ? Math.round(s * 1000) : null;
}

/** The body of `PUT /__kaanal/chaos` for an action on a target. */
export function chaosBody(target: ChaosTarget, action: ChaosAction): Record<string, unknown> {
  switch (action.kind) {
    case "down":
      return {
        ...target,
        down: action.down,
        // the start of an outage (RULES 91): only taking a node or instance down has one
        ...(action.down && action.fromMs !== undefined && "node" in target
          ? { from_ms: action.fromMs }
          : {}),
      };
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

/** A node's network card in KB/s (1024 bytes), `{ send, receive }`; null = no limit that way. */
export function cardOf(config: NodeDoc): { send: number | null; receive: number | null } {
  const kb = (v: unknown) => {
    const bytes = parseBytes(v, true);
    return bytes === null ? null : Number((bytes / 1024).toFixed(3));
  };
  const b = config.bandwidth;
  if (isObject(b)) return { send: kb(b.out), receive: kb(b.in) };
  return { send: kb(b), receive: kb(b) };
}

/**
 * The patch for one side of the card set to `kb` KB/s (null removes that side). The file keeps one
 * number when both sides are equal and `{ out, in }` otherwise; no side left removes the field.
 */
export function setCard(
  name: string,
  config: NodeDoc,
  side: "send" | "receive",
  kb: number | null,
): Op[] {
  const path = ["nodes", name, "bandwidth"];
  const now = cardOf(config);
  const next = { ...now, [side]: kb };
  const bytes = (v: number | null) => (v === null ? null : Math.max(1, Math.round(v * 1024)));
  const [out, rx] = [bytes(next.send), bytes(next.receive)];
  const present = "bandwidth" in config;
  if (out === null && rx === null) return present ? [{ op: "remove", path }] : [];
  const value = out === rx ? out : { ...(out !== null && { out }), ...(rx !== null && { in: rx }) };
  return [put(path, present, value)];
}

/** The kinds whose compute the person sizes, so they take `cpu` (not a cache, an external API or a store). */
export const CPU_KINDS: readonly string[] = [
  "gateway",
  "service",
  "monolith",
  "database",
  "auth",
  "load-balancer",
];
export const hasCpu = (type: string): boolean => CPU_KINDS.includes(type);

/** The cores a component shares its own work over (`cpu: { cores }`); null = no sharing. */
export function coresOf(config: NodeDoc): number | null {
  const c = config.cpu;
  const n = isObject(c) ? c.cores : null;
  return typeof n === "number" && Number.isFinite(n) && n > 0 ? n : null;
}

/** The patch for the cores box: a number writes `cpu: { cores }`, null removes the field. */
export function setCores(name: string, config: NodeDoc, cores: number | null): Op[] {
  const path = ["nodes", name, "cpu"];
  const present = "cpu" in config;
  if (cores === null || !(cores > 0)) return present ? [{ op: "remove", path }] : [];
  return [put(path, present, { cores })];
}
