// The machine fields of a node as data: its network card, cores, instance, memory, GC and what
// happens at the memory limit. Split out of fields.ts (re-exported from it, so callers do not change).

import type { Doc, NodeDoc, Op } from "../../data/api/types";
import { durationMs, formatMs, isObject, put } from "./fields.base";
import { parseBytes } from "./units";

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

/** The instance type a component names (`instance: aws/m5.large`); null = none. */
export function instanceOf(config: NodeDoc): string | null {
  const v = config.instance;
  return typeof v === "string" && v.trim() !== "" ? v : null;
}

/** The patch for the instance picker: a name writes `instance`, null removes it. */
export function setInstance(name: string, config: NodeDoc, value: string | null): Op[] {
  const path = ["nodes", name, "instance"];
  const present = "instance" in config;
  if (value === null || value === "") return present ? [{ op: "remove", path }] : [];
  return [put(path, present, value)];
}

/** One of the file's own instance types (`instances:`), as the boxes show it. */
export interface CustomInstance {
  cores: number | null;
  /** Bytes; null = none. */
  memory: number | null;
  /** Bytes a second, both ways; null = not limited (or written as `{ out, in }`). */
  bandwidth: number | null;
}

/** The names a custom instance may take: no `/` (the cloud table is `provider/type`). */
export const CUSTOM_NAME = /^[A-Za-z0-9._-]{1,64}$/;

/** The file's own instance types by name, as numbers where the file wrote numbers. */
export function customInstancesOf(doc: Doc | null | undefined): Record<string, CustomInstance> {
  const raw = isObject(doc?.instances) ? doc.instances : {};
  const num = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v > 0 ? v : null);
  const out: Record<string, CustomInstance> = {};
  for (const [name, v] of Object.entries(raw)) {
    const e = isObject(v) ? v : {};
    out[name] = { cores: num(e.cores), memory: num(e.memory), bandwidth: num(e.bandwidth) };
  }
  return out;
}

/** The patch that adds a custom instance (the `instances:` map too, when the file has none). */
export function addCustomInstance(doc: Doc | null, name: string, cores: number): Op[] {
  const entry = { cores };
  if (!isObject(doc?.instances)) {
    return [{ op: "insert", path: ["instances"], value: { [name]: entry } }];
  }
  return [{ op: "insert", path: ["instances", name], value: entry }];
}

/**
 * The patch for one box of a custom instance: a number writes the key, 0 or null takes it out
 * (cores stay: an instance without CPUs is not one). A key the file wrote as text is kept.
 */
export function setCustomInstance(
  doc: Doc | null,
  name: string,
  key: "cores" | "memory" | "bandwidth",
  value: number | null,
): Op[] {
  const all = doc?.instances;
  const raw = isObject(all) && isObject(all[name]) ? all[name] : null;
  if (raw === null) return [];
  const next: Record<string, unknown> = { ...raw };
  if (value === null || !(value > 0)) {
    if (key === "cores") return [];
    delete next[key];
  } else {
    next[key] = key === "cores" ? value : Math.round(value);
  }
  return [{ op: "set", path: ["instances", name], value: next }];
}

/** The nodes that name a custom instance. */
export function usersOfInstance(doc: Doc | null | undefined, name: string): string[] {
  return Object.entries(doc?.nodes ?? {})
    .filter(([, n]) => n.instance === name)
    .map(([k]) => k);
}

/** The patch that deletes a custom instance; none while a node still names it. */
export function removeCustomInstance(doc: Doc | null, name: string): Op[] {
  if (usersOfInstance(doc, name).length > 0) return [];
  return [{ op: "remove", path: ["instances", name] }];
}

/** The three sizes of `memory: { limit, per_request, base }`, in bytes; null where not a number. */
export type MemoryKey = "limit" | "per_request" | "base";
export function memoryOf(config: NodeDoc): Record<MemoryKey, number | null> {
  const m = isObject(config.memory) ? config.memory : {};
  const n = (k: MemoryKey) => {
    const v = m[k];
    return typeof v === "number" && Number.isFinite(v) && v > 0 ? v : null;
  };
  return { limit: n("limit"), per_request: n("per_request"), base: n("base") };
}

/**
 * The patch for one memory box (bytes): the field is set or left out and the map is written
 * whole; no field left removes `memory`. A size written as text in the file (`8GB`) is kept as
 * it is for the boxes not touched.
 */
export function setMemory(
  name: string,
  config: NodeDoc,
  key: MemoryKey,
  bytes: number | null,
): Op[] {
  const path = ["nodes", name, "memory"];
  const present = "memory" in config;
  const next: Record<string, unknown> = isObject(config.memory) ? { ...config.memory } : {};
  if (bytes === null || !(bytes > 0)) delete next[key];
  else next[key] = Math.round(bytes);
  if (Object.keys(next).length === 0) return present ? [{ op: "remove", path }] : [];
  return [put(path, present, next)];
}

/** The garbage-collection weight of `memory.gc`, or null when none is set. */
export function gcOf(config: NodeDoc): number | null {
  const v = isObject(config.memory) ? config.memory.gc : null;
  return typeof v === "number" && Number.isFinite(v) && v > 0 ? v : null;
}

/** The patch for the GC box: a number writes `memory.gc`, null takes it out. */
export function setGc(name: string, config: NodeDoc, gc: number | null): Op[] {
  if (!isObject(config.memory)) return [];
  const next: Record<string, unknown> = { ...config.memory };
  if (gc === null || !(gc > 0)) delete next.gc;
  else next.gc = gc;
  return [put(["nodes", name, "memory"], true, next)];
}

/** What a copy does with a request its memory cannot hold (`memory.on_limit`). */
export type OnLimit = "refuse" | "kill";
/** How long a killed copy stays down when the file says nothing (`memory.restart`). */
export const DEFAULT_RESTART_MS = 10_000;

export function onLimitOf(config: NodeDoc): OnLimit {
  return isObject(config.memory) && config.memory.on_limit === "kill" ? "kill" : "refuse";
}

/** The restart delay in ms, the default when the file leaves it out; null for text it cannot read. */
export function restartMsOf(config: NodeDoc): number | null {
  if (!isObject(config.memory) || !("restart" in config.memory)) return DEFAULT_RESTART_MS;
  return durationMs(config.memory.restart);
}

/**
 * The patch for the "when memory runs out" choice. `kill` writes `on_limit: kill`; `refuse` takes
 * `on_limit` and `restart` out (the file refuses a `restart` that has nothing to restart).
 */
export function setOnLimit(name: string, config: NodeDoc, mode: OnLimit): Op[] {
  const path = ["nodes", name, "memory"];
  if (!isObject(config.memory)) return [];
  const next: Record<string, unknown> = { ...config.memory };
  if (mode === "kill") next.on_limit = "kill";
  else {
    delete next.on_limit;
    delete next.restart;
  }
  return [put(path, true, next)];
}

/** The patch for the restart box (ms): written as the loader reads it. */
export function setRestart(name: string, config: NodeDoc, ms: number): Op[] {
  const path = ["nodes", name, "memory"];
  if (!isObject(config.memory)) return [];
  return [put(path, true, { ...config.memory, restart: formatMs(ms) })];
}

/** The patch for the cores box: a number writes `cpu: { cores }`, null removes the field. */
export function setCores(name: string, config: NodeDoc, cores: number | null): Op[] {
  const path = ["nodes", name, "cpu"];
  const present = "cpu" in config;
  if (cores === null || !(cores > 0)) return present ? [{ op: "remove", path }] : [];
  return [put(path, present, { cores })];
}
