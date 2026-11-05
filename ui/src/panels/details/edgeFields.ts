// A wire's settings as data: where each lives in the config and the patch that edits it, as
// `fields.ts` does for a node. The loader's limits are in `EDGE_LIMITS`; a value outside them would
// make the draft invalid, so the fields never send one. `net` written as a spread (p50 and p99, a
// mean) is shown but not edited here.

import type { Doc, EdgeDoc, Op } from "../../data/api/types";
import { durationMs, formatMs } from "./fields";
import { parseBytes } from "./units";

export const EDGE_LIMITS = {
  netMs: { min: 0, max: 60_000 },
  /** The loader caps a repeated call at 1000. */
  calls: { min: 1, max: 1000 },
  /** Percent of requests that take the wire; above 0. */
  sharePct: { min: 0.01, max: 100 },
  bandwidthKb: { min: 0.001, max: 1_000_000_000 },
} as const;

const put = (i: number, field: string, e: EdgeDoc, value: unknown): Op => ({
  op: field in e ? "set" : "insert",
  path: ["edges", i, field],
  value,
});

/** The wire's network latency when it is a plain duration; null when absent or a spread. */
export function netMsOf(e: EdgeDoc): { ms: number | null; spread: boolean } {
  if (!("net" in e)) return { ms: null, spread: false };
  const ms = durationMs(e.net);
  return { ms, spread: ms === null };
}

export const setNet = (i: number, e: EdgeDoc, ms: number): Op => put(i, "net", e, formatMs(ms));

export const callsOf = (e: EdgeDoc): number => (typeof e.calls === "number" ? e.calls : 1);
export const setCalls = (i: number, e: EdgeDoc, n: number): Op => put(i, "calls", e, n);

/** The share of requests that take the wire, in percent (the config's 0 to 1, times 100). */
export const shareOf = (e: EdgeDoc): number =>
  typeof e.prob === "number" ? Number((e.prob * 100).toFixed(4)) : 100;
export const setShare = (i: number, e: EdgeDoc, pct: number): Op =>
  put(i, "prob", e, Number((pct / 100).toFixed(6)));

/**
 * Whether the config accepts `prob` here: not on a wire out of a gateway, and not on one into a
 * cache (a cache keeps its hit ratio). An unknown end is treated as allowed.
 */
export function shareApplies(doc: Doc | null | undefined, e: EdgeDoc): boolean {
  const kind = (n: unknown) => doc?.nodes?.[String(n)]?.type;
  return kind(e.from) !== "gateway" && kind(e.to) !== "cache";
}

/** The link's capacity in KB/s (1024 bytes), or null when there is no limit. */
export const bandwidthKbOf = (e: EdgeDoc): number | null => {
  const bytes = parseBytes(e.bandwidth, true);
  return bytes === null ? null : Number((bytes / 1024).toFixed(3));
};
/** Bytes per second, at least 1 and whole, as the loader reads them. */
export const setBandwidth = (i: number, e: EdgeDoc, kb: number): Op =>
  put(i, "bandwidth", e, Math.max(1, Math.round(kb * 1024)));
export const clearBandwidth = (i: number, e: EdgeDoc): Op[] =>
  "bandwidth" in e ? [{ op: "remove", path: ["edges", i, "bandwidth"] }] : [];

/** The loader's cap on a body size: 1 GiB. */
export const BODY_MAX_BYTES = 1024 ** 3;

type Body = "sends" | "returns";

/** The body size a call over the wire carries (`sends`) or brings back (`returns`), in bytes; null = inherited. */
export const bodyBytesOf = (e: EdgeDoc, field: Body): number | null => parseBytes(e[field]);
/** Whole bytes, as the loader reads them. */
export const setBody = (i: number, field: Body, e: EdgeDoc, bytes: number): Op =>
  put(i, field, e, Math.round(bytes));
export const clearBody = (i: number, field: Body, e: EdgeDoc): Op[] =>
  field in e ? [{ op: "remove", path: ["edges", i, field] }] : [];

/** What the file assumes for the headers of a call when `header_bytes` is not written (RULES 65). */
export const DEFAULT_HEADER_BYTES = { request: 600, response: 400 } as const;
export type HeaderSide = keyof typeof DEFAULT_HEADER_BYTES;

/** Header bytes of a request and a response: the file's `header_bytes` (one size, or a pair), else the defaults. */
export function headerBytesOf(doc: Doc | null | undefined): Record<HeaderSide, number> {
  const v = doc?.header_bytes;
  if (v !== null && typeof v === "object" && !Array.isArray(v)) {
    const m = v as Record<string, unknown>;
    return {
      request: parseBytes(m.request) ?? DEFAULT_HEADER_BYTES.request,
      response: parseBytes(m.response) ?? DEFAULT_HEADER_BYTES.response,
    };
  }
  const both = parseBytes(v);
  return both === null ? { ...DEFAULT_HEADER_BYTES } : { request: both, response: both };
}

/** Sets one side, writing the pair so the other side keeps what it shows. */
export const setHeaderBytes = (
  doc: Doc | null | undefined,
  side: HeaderSide,
  bytes: number,
): Op => ({
  op: doc && "header_bytes" in doc ? "set" : "insert",
  path: ["header_bytes"],
  value: { ...headerBytesOf(doc), [side]: Math.round(bytes) },
});

/** The TLS a new connection over the wire negotiates (RULES 68): none, 1.3 (one extra round trip) or 1.2 (two). */
export type TlsChoice = "off" | "1.3" | "1.2";
export const TLS_CHOICES: readonly { value: TlsChoice; label: string }[] = [
  { value: "off", label: "None (plain TCP)" },
  { value: "1.3", label: "TLS 1.3" },
  { value: "1.2", label: "TLS 1.2" },
];

/** What the file's `tls` means: `true` and 1.3 are 1.3; anything the loader would refuse reads as none. */
export function tlsOf(e: EdgeDoc): TlsChoice {
  const v = e.tls;
  if (v === true || v === 1.3 || v === "1.3") return "1.3";
  if (v === 1.2 || v === "1.2") return "1.2";
  return "off";
}
/** Off removes the key (the default); a version writes the number. */
export const setTls = (i: number, e: EdgeDoc, c: TlsChoice): Op[] =>
  c === "off"
    ? "tls" in e
      ? [{ op: "remove", path: ["edges", i, "tls"] }]
      : []
    : [put(i, "tls", e, c === "1.3" ? 1.3 : 1.2)];

/** Calls a keep-alive connection serves before it closes: the loader's range and its default. */
export const KEEPALIVE = { min: 1, max: 1_000_000, default: 1000 } as const;
/** The file's `keepalive`, or null when it is not written (the default of 1000 applies). */
export const keepaliveOf = (e: EdgeDoc): number | null =>
  typeof e.keepalive === "number" && Number.isInteger(e.keepalive) ? e.keepalive : null;
export const setKeepalive = (i: number, e: EdgeDoc, n: number): Op => put(i, "keepalive", e, n);
export const clearKeepalive = (i: number, e: EdgeDoc): Op[] =>
  "keepalive" in e ? [{ op: "remove", path: ["edges", i, "keepalive"] }] : [];

/** Which bodies of a call over the wire stream (RULES 71, 76): the callee's default, both, one, or neither. */
export type StreamChoice = "default" | "both" | "response" | "request" | "none";
export const STREAM_CHOICES: readonly { value: StreamChoice; label: string }[] = [
  { value: "default", label: "Default (a storage read or write streams)" },
  { value: "both", label: "Both, upload and download" },
  { value: "response", label: "Download only (the response)" },
  { value: "request", label: "Upload only (the request)" },
  { value: "none", label: "Neither, buffer both" },
];
/** The file's `stream`: true is both, false is neither, or the word. */
export function streamOf(e: EdgeDoc): StreamChoice {
  const v = e.stream;
  if (v === true || v === "both" || v === "true") return "both";
  if (v === false || v === "none" || v === "false") return "none";
  if (v === "response" || v === "request") return v;
  return "default";
}
/** The default removes the key; both and none write the boolean, a direction writes the word. */
export const setStream = (i: number, e: EdgeDoc, c: StreamChoice): Op[] =>
  c === "default"
    ? "stream" in e
      ? [{ op: "remove", path: ["edges", i, "stream"] }]
      : []
    : [put(i, "stream", e, c === "both" ? true : c === "none" ? false : c)];

/** Packet loss on the wire (RULES 74): the loader's 0 to 1, shown as a percent; 0 or absent is off. */
export const LOSS_LIMITS = { min: 0, max: 100 } as const;
export const lossPctOf = (e: EdgeDoc): number | null =>
  typeof e.loss === "number" && e.loss > 0 ? Number((e.loss * 100).toFixed(4)) : null;
/** Zero removes the key (off); anything else writes the probability. */
export const setLoss = (i: number, e: EdgeDoc, pct: number): Op[] =>
  pct <= 0
    ? "loss" in e
      ? [{ op: "remove", path: ["edges", i, "loss"] }]
      : []
    : [put(i, "loss", e, Number((pct / 100).toFixed(6)))];

/** The share of its size a compressed response keeps on the wire, as a percent; null = not compressed. */
export const COMPRESS_LIMITS = { min: 0.1, max: 100 } as const;
export const compressPctOf = (e: EdgeDoc): number | null =>
  typeof e.compress === "number" && e.compress > 0 && e.compress <= 1
    ? Number((e.compress * 100).toFixed(4))
    : null;
export const setCompress = (i: number, e: EdgeDoc, pct: number): Op =>
  put(i, "compress", e, Number((pct / 100).toFixed(6)));
export const clearCompress = (i: number, e: EdgeDoc): Op[] =>
  "compress" in e ? [{ op: "remove", path: ["edges", i, "compress"] }] : [];
