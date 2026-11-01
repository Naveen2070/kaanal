// The traffic builder's model (developer, 2026-11-01): the active traffic profile as rows, one per
// endpoint a request can ask for, and the patch that writes the rows back. Pure, so it is tested
// without a dialog.
//
// A row is a method and a path, a weight in the mix, payload sizes, and which services sit behind
// the gateway for it. Several services are called in parallel (`parallel:` in the endpoint's chain).
// A service the gateway has no edge to gets one when the rows are applied. A chain the builder
// cannot show as a list of services (one a person wrote by hand) is kept as it is.

import type { Doc, EdgeDoc, NodeDoc, Op } from "../api/types";

export const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE"] as const;
export type Method = (typeof METHODS)[number];

export const DEFAULT_PROFILE = "peak";
export const DEFAULT_RPS = 100;
/** Both sizes are held to the config's own limit of 1 GiB. */
export const MAX_KB = 1024 * 1024;

export interface Row {
  /** The endpoint key the row came from (`"GET /search"`), or null for a new row. */
  from: string | null;
  method: Method;
  path: string;
  weight: number;
  reqKb: number;
  respKb: number;
  /** Services behind the gateway, or null for a chain written by hand (kept as it is). */
  targets: string[] | null;
}

type Obj = Record<string, unknown>;
const isObj = (x: unknown): x is Obj => x !== null && typeof x === "object" && !Array.isArray(x);

export const keyOf = (method: string, path: string): string => `${method} ${path}`;

/** The profile the offered-load slider drives: the first one, or the default name with none. */
export function activeProfile(doc: Doc | null | undefined): string {
  const t = doc?.traffic;
  return isObj(t) && Object.keys(t)[0] !== undefined
    ? (Object.keys(t)[0] as string)
    : DEFAULT_PROFILE;
}

export const gatewayOf = (doc: Doc | null | undefined): string | null =>
  Object.entries(doc?.nodes ?? {}).find(([, n]) => n.type === "gateway")?.[0] ?? null;

/** What a request can be sent to: services and monoliths. */
export function serviceNames(doc: Doc | null | undefined): string[] {
  return Object.entries(doc?.nodes ?? {})
    .filter(([, n]: [string, NodeDoc]) => n.type === "service" || n.type === "monolith")
    .map(([name]) => name);
}

/** A step that calls a service; it may carry its own inner chain, `op`, `repeat` and so on. */
const callOf = (s: unknown): string | null =>
  isObj(s) && typeof s.call === "string" ? s.call : null;

/** The call steps of a chain: its list, or the list inside a lone `parallel:`; null when it is not just calls. */
function callSteps(chain: unknown): Obj[] | null {
  if (chain === undefined) return [];
  if (!Array.isArray(chain)) return null;
  const list =
    chain.length === 1 && isObj(chain[0]) && Array.isArray(chain[0].parallel)
      ? (chain[0].parallel as unknown[])
      : chain;
  return list.every((st) => callOf(st) !== null) ? (list as Obj[]) : null;
}

/**
 * The services a chain calls first, when the chain is only calls (alone or in one `parallel:`),
 * whatever they do inside; null for one with other steps (a cache, say), which is kept as written.
 */
export function targetsOfChain(chain: unknown): string[] | null {
  return callSteps(chain)?.map((st) => callOf(st) as string) ?? null;
}

const toKb = (bytes: unknown) => (typeof bytes === "number" && bytes > 0 ? bytes / 1024 : 0);

/** The rows of `profile`: its journeys, each with what the endpoint of that name does. */
export function rowsFrom(doc: Doc | null | undefined, profile: string): Row[] {
  const t = doc?.traffic;
  const journeys =
    isObj(t) && isObj(t[profile]) && Array.isArray((t[profile] as Obj).journeys)
      ? ((t[profile] as Obj).journeys as unknown[])
      : [];
  const endpoints = isObj(doc?.endpoints) ? (doc?.endpoints as Obj) : {};
  return journeys.filter(isObj).map((j) => {
    const key = typeof j.endpoint === "string" ? j.endpoint : "";
    const [m, ...rest] = key.split(" ");
    const method = (METHODS as readonly string[]).includes(m ?? "") ? (m as Method) : "GET";
    const e = endpoints[key];
    return {
      from: key,
      method,
      path: rest.join(" "),
      weight: typeof j.weight === "number" ? j.weight : 1,
      reqKb: toKb(j.req_bytes),
      respKb: toKb(j.resp_bytes),
      targets: isObj(e) ? targetsOfChain(e.chain) : [],
    };
  });
}

export const blankRow = (): Row => ({
  from: null,
  method: "GET",
  path: "/",
  weight: 1,
  reqKb: 0,
  respKb: 0,
  targets: [],
});

/** What is wrong with the rows, in words, one line per problem; empty when they can be applied. */
export function problems(rows: Row[], doc: Doc | null | undefined): string[] {
  const out: string[] = [];
  if (rows.length === 0) out.push("Add at least one endpoint.");
  if (rows.length > 64) out.push("A profile holds at most 64 endpoints.");
  const seen = new Map<string, number>();
  rows.forEach((r, i) => {
    const n = i + 1;
    if (!r.path.startsWith("/") || /\s/.test(r.path)) {
      out.push(`Endpoint ${n}: the path starts with / and has no spaces.`);
    }
    if (!(r.weight > 0) || !Number.isFinite(r.weight)) {
      out.push(`Endpoint ${n}: the weight is above 0.`);
    }
    for (const [what, kb] of [
      ["request", r.reqKb],
      ["response", r.respKb],
    ] as const) {
      if (!(kb >= 0) || kb > MAX_KB || !Number.isFinite(kb)) {
        out.push(`Endpoint ${n}: the ${what} size is 0 to ${MAX_KB.toLocaleString("en-US")} KB.`);
      }
    }
    const k = keyOf(r.method, r.path).toLowerCase();
    if (seen.has(k))
      out.push(`Endpoint ${n} repeats endpoint ${seen.get(k)}: give it a bigger weight instead.`);
    else seen.set(k, n);
  });
  if (isObj(doc) && typeof doc.endpoints === "string") {
    out.push("The endpoints are in a separate file: edit them there.");
  }
  if (gatewayOf(doc) === null && rows.some((r) => (r.targets ?? []).length > 0)) {
    out.push("Add a gateway first: it is what routes a request to a service.");
  }
  return out;
}

const bytes = (kb: number) => Math.round(kb * 1024);
const opFor = (m: Method) => (m === "GET" ? "read" : "write");

/** The endpoint entry for a row (chain, status and op); a hand-written chain is not touched here. */
function endpointOf(r: Row, old: unknown): Obj {
  const prev = isObj(old) ? old : {};
  const entry: Obj = { ...prev, op: prev.op ?? opFor(r.method) };
  if (!isObj(entry.override)) entry.override = { status: r.method === "POST" ? 201 : 200 };
  if (r.targets !== null) {
    // a service that stays keeps its step as written (its inner chain, op and repeat)
    const before = new Map((callSteps(prev.chain) ?? []).map((st) => [callOf(st) as string, st]));
    const calls = r.targets.map((t) => before.get(t) ?? { call: t });
    entry.chain = calls.length <= 1 ? calls : [{ parallel: calls }];
    if (calls.length === 0) delete entry.chain;
  }
  return entry;
}

/**
 * The patch that makes `doc` say what `rows` say: the profile's journeys, one endpoint entry per
 * row, and an edge from the gateway to every service a row sends to that has none yet. One patch,
 * so one undo. A row that was removed leaves its endpoint in the file (it is still a route).
 */
export function buildOps(doc: Doc, profile: string, rows: Row[]): Op[] {
  const ops: Op[] = [];
  const gateway = gatewayOf(doc);

  // endpoints
  const existing = isObj(doc.endpoints) ? (doc.endpoints as Obj) : null;
  const entries: [string, Obj][] = rows.map((r) => {
    const key = keyOf(r.method, r.path);
    // a renamed row starts from what the old name did, so its chain and reply carry over
    return [key, endpointOf(r, existing?.[key] ?? (r.from ? existing?.[r.from] : undefined))];
  });
  if (existing === null) {
    ops.push({
      op: "endpoints" in doc ? "set" : "insert",
      path: ["endpoints"],
      value: Object.fromEntries(entries),
    });
  } else {
    for (const [key, value] of entries) {
      ops.push({ op: key in existing ? "set" : "insert", path: ["endpoints", key], value });
    }
  }

  // the mix
  const journeys = rows.map((r) => ({
    endpoint: keyOf(r.method, r.path),
    weight: r.weight,
    ...(r.reqKb > 0 && { req_bytes: bytes(r.reqKb) }),
    ...(r.respKb > 0 && { resp_bytes: bytes(r.respKb) }),
  }));
  const traffic = isObj(doc.traffic) ? (doc.traffic as Obj) : null;
  const current = traffic && isObj(traffic[profile]) ? (traffic[profile] as Obj) : null;
  if (current) {
    ops.push({
      op: "journeys" in current ? "set" : "insert",
      path: ["traffic", profile, "journeys"],
      value: journeys,
    });
  } else if (traffic) {
    ops.push({ op: "insert", path: ["traffic", profile], value: { rps: DEFAULT_RPS, journeys } });
  } else {
    ops.push({
      op: "traffic" in doc ? "set" : "insert",
      path: ["traffic"],
      value: { [profile]: { rps: DEFAULT_RPS, journeys } },
    });
  }

  // the gateway's edges
  if (gateway !== null) {
    const have = new Set(
      (doc.edges ?? []).filter((e) => String(e.from) === gateway).map((e) => String(e.to)),
    );
    const wanted = [...new Set(rows.flatMap((r) => r.targets ?? []))].filter((t) => !have.has(t));
    const fresh: EdgeDoc[] = wanted.map((to) => ({ from: gateway, to }));
    if (fresh.length > 0) {
      if (Array.isArray(doc.edges)) {
        for (const e of fresh) ops.push({ op: "append", path: ["edges"], value: e });
      } else {
        ops.push({ op: "edges" in doc ? "set" : "insert", path: ["edges"], value: fresh });
      }
    }
  }
  return ops;
}

/** The services a set of rows would newly connect to the gateway (shown before it is applied). */
export function newConnections(doc: Doc | null | undefined, rows: Row[]): string[] {
  const gateway = gatewayOf(doc);
  if (gateway === null) return [];
  const have = new Set(
    (doc?.edges ?? []).filter((e) => String(e.from) === gateway).map((e) => String(e.to)),
  );
  return [...new Set(rows.flatMap((r) => r.targets ?? []))].filter((t) => !have.has(t));
}
