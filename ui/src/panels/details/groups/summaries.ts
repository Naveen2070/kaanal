// What each folded group of a wire's Controls says about itself, and which keys of the file belong
// to it ("changed" = one of them is written; Reset removes them). Pure functions of the edge, so a
// closed group shows the current setting without opening. The numbers are the loader's defaults,
// read from the same constants the boxes use (`resilience.ts`, `edgeFields.ts`), never copied.

import type { Doc, EdgeDoc, NodeDoc, Op } from "../../../data/api/types";
import { balanceOf, cardOf, holdOf, rateLimitOf } from "../fields";
import { rateText } from "../units";
import {
  callsOf,
  compressPctOf,
  headerBytesOf,
  keepaliveOf,
  lossPctOf,
  shareOf,
  streamOf,
  tlsOf,
} from "../edgeFields";
import {
  abandonOf,
  attemptsOf,
  backoffMsOf,
  BREAKER,
  breakerOf,
  budgetOf,
  detectMsOf,
  multiplierOf,
  openForMsOf,
  ratePctOf,
  RETRY,
  retryOf,
  windowOf,
} from "../resilience";

/** 50 ms, 60 s, 2 min: how a person says a duration (a minute stays "60 s", the loader's own word). */
export function humanMs(ms: number): string {
  const n = (v: number) => Number(v.toFixed(2));
  if (ms > 60_000) return `${n(ms / 60_000)} min`;
  if (ms >= 1000) return `${n(ms / 1000)} s`;
  return `${n(ms)} ms`;
}

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many);

export type WireGroup = "connection" | "compress" | "retry" | "breaker" | "usage";

/** The keys of an edge that each group owns. */
export const GROUP_KEYS: Record<WireGroup, readonly string[]> = {
  connection: ["tls", "keepalive", "loss"],
  compress: ["compress", "stream"],
  retry: ["retry", "abandon"],
  breaker: ["breaker"],
  usage: ["calls", "prob", "balance", "detect"],
};

export const wireGroupChanged = (g: WireGroup, e: EdgeDoc): boolean =>
  GROUP_KEYS[g].some((k) => k in e);

/** Removes every key of the group that is written, as one patch (one undo step). */
export const resetOps = (g: WireGroup, index: number, e: EdgeDoc): Op[] =>
  GROUP_KEYS[g].filter((k) => k in e).map((k): Op => ({ op: "remove", path: ["edges", index, k] }));

export function connectionSummary(e: EdgeDoc): string {
  const parts: string[] = [];
  const tls = tlsOf(e);
  if (tls !== "off") parts.push(`TLS ${tls}`);
  const keep = keepaliveOf(e);
  if (keep !== null) parts.push(`${keep} ${plural(keep, "call", "calls")} a connection`);
  const loss = lossPctOf(e);
  if (loss !== null) parts.push(`${loss}% loss`);
  return parts.length ? parts.join(" · ") : "defaults";
}

export function compressStreamSummary(e: EdgeDoc): string {
  const parts: string[] = [];
  const keeps = compressPctOf(e);
  if (keeps !== null) parts.push(`keeps ${keeps}%`);
  const s = streamOf(e);
  if (s !== "default") {
    parts.push(
      {
        both: "streamed",
        none: "buffered",
        response: "response streamed",
        request: "request streamed",
      }[s],
    );
  }
  return parts.length ? parts.join(" · ") : "defaults";
}

export function retrySummary(e: EdgeDoc): string {
  const r = retryOf(e);
  const goes = abandonOf(e) === "continue" ? "work continues" : null;
  if (!r) return goes ? `retries off · ${goes}` : "off";
  const wait = backoffMsOf(r) ?? RETRY.backoffMs.default;
  const parts = [`${attemptsOf(r)} tries`, `${humanMs(wait)} ×${multiplierOf(r)}`];
  if (budgetOf(r)) parts.push("budget on");
  if (goes) parts.push(goes);
  return parts.join(" · ");
}

export function breakerSummary(e: EdgeDoc): string {
  const b = breakerOf(e);
  if (!b) return "off";
  const wait = openForMsOf(b) ?? BREAKER.openForMs.default;
  return `${windowOf(b)} calls · opens at ${ratePctOf(b)}% · ${humanMs(wait)}`;
}

export function usageSummary(e: EdgeDoc): string {
  const parts: string[] = [];
  if ("calls" in e) parts.push(`${callsOf(e)} ${plural(callsOf(e), "call", "calls")} a request`);
  if ("prob" in e) parts.push(`${shareOf(e)}% of requests`);
  const b = balanceOf(e.balance);
  if (b) parts.push(b === "weighted" ? "weighted" : b);
  const d = detectMsOf(e as Record<string, unknown>);
  if (d !== null) parts.push(`notices after ${humanMs(d)}`);
  return parts.length ? parts.join(" · ") : "defaults";
}

/** The titles of the groups that hold a setting, in the panel's order, for the wire's header line. */
export const WIRE_GROUP_TITLES: Record<WireGroup, string> = {
  connection: "Connection",
  retry: "When a call fails",
  breaker: "Circuit breaker",
  compress: "Compression and streaming",
  usage: "How often it is used",
};

export function changedLine(e: EdgeDoc): string {
  const order: WireGroup[] = ["connection", "retry", "breaker", "compress", "usage"];
  const titles = order.filter((g) => wireGroupChanged(g, e)).map((g) => WIRE_GROUP_TITLES[g]);
  return titles.length
    ? `Changed from the defaults: ${titles.join(", ")}`
    : "Nothing changed from the defaults.";
}

/** A component's network card, for its closed group. */
export function nodeCardSummary(config: NodeDoc): string {
  const { send, receive } = cardOf(config);
  const parts: string[] = [];
  if (send !== null) parts.push(`sends up to ${rateText(send)}`);
  if (receive !== null) parts.push(`receives up to ${rateText(receive)}`);
  return parts.length ? parts.join(" · ") : "no limit";
}

// ---- a component's groups ----

export type NodeGroup = "limit" | "balance" | "style";

/** The keys of a component that each group owns. */
export const NODE_GROUP_KEYS: Record<NodeGroup, readonly string[]> = {
  limit: ["rate_limit"],
  balance: ["balance", "spread", "detect"],
  style: ["hold"],
};

export const nodeGroupChanged = (g: NodeGroup, c: NodeDoc): boolean =>
  NODE_GROUP_KEYS[g].some((k) => k in c);

/** Removes every key of the group that is written, as one patch (one undo step). */
export const nodeResetOps = (g: NodeGroup, name: string, c: NodeDoc): Op[] =>
  NODE_GROUP_KEYS[g]
    .filter((k) => k in c)
    .map((k): Op => ({ op: "remove", path: ["nodes", name, k] }));

const count = (n: number) => n.toLocaleString("en-US");

export function limitSummary(c: NodeDoc): string {
  const l = rateLimitOf(c);
  if (l) return `${count(l.rps)} / s · burst ${count(l.burst)}`;
  return "rate_limit" in c ? "set in kaanal.yaml" : "none";
}

export const styleSummary = (c: NodeDoc): string =>
  holdOf(c) === "call" ? "thread per request" : "event loop";

const BALANCE_WORDS: Record<string, string> = {
  "round-robin": "round robin",
  random: "random",
  "least-conn": "least connections",
  p2c: "power of two choices",
  weighted: "weighted",
};
export const balanceWord = (v: unknown) =>
  BALANCE_WORDS[balanceOf(v) ?? "round-robin"] ?? "round robin";

export function balanceSummary(c: NodeDoc): string {
  const parts: string[] = [];
  const lb = c.type === "load-balancer";
  if (lb) parts.push(`spread ${balanceWord(c.spread)}`);
  if (!lb || Number(c.replicas ?? 1) > 1) {
    if (!lb || "balance" in c) parts.push(balanceWord(c.balance));
  }
  const d = detectMsOf(c as Record<string, unknown>);
  if (d !== null) parts.push(`notices after ${humanMs(d)}`);
  return parts.join(" · ");
}

// ---- the file's headers ----

/** 600 B, 1 KB, 1.5 MB: a byte count as a person says it (KB is 1024, as everywhere in Controls). */
export function byteText(bytes: number): string {
  const n = (v: number) => Number(v.toFixed(2));
  if (bytes >= 1024 ** 3) return `${n(bytes / 1024 ** 3)} GB`;
  if (bytes >= 1024 ** 2) return `${n(bytes / 1024 ** 2)} MB`;
  if (bytes >= 1024) return `${n(bytes / 1024)} KB`;
  return `${bytes} B`;
}

export function headersSummary(doc: Doc | null | undefined): string {
  const h = headerBytesOf(doc);
  if (h.request === 0 && h.response === 0) return "off";
  return `${byteText(h.request)} request · ${byteText(h.response)} response`;
}
