// A thin fetch wrapper for the control API. Every call sends the bearer token, can be aborted, and
// fails as an `ApiError`; a busy simulator is a result, not a failure (`startRun`).

import { readToken } from "../../token";
import type {
  ApplyResult,
  Diagnostic,
  DraftView,
  LogPage,
  LivePin,
  LogQuery,
  PinBody,
  PinsView,
  PutBody,
  RunBody,
  RunRecord,
  SaveBody,
  SampleResult,
  SaveResult,
  ScenarioRow,
  Topology,
  ValidateResult,
} from "./types";
import type { FeedRow } from "./logFeed";
import type { PayloadEdit, PayloadPreview } from "./payload";

const ROOT = "/__kaanal";

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
    readonly diagnostic?: Diagnostic,
    readonly body?: unknown,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

export type StartRun =
  | { kind: "started"; id: string }
  | { kind: "busy"; status: 409 | 429; message: string };

interface Opts {
  signal?: AbortSignal;
}

const token = readToken;

/** Sends one request and returns the status and the parsed JSON body (null when there is none). */
async function send(
  method: string,
  path: string,
  body: unknown,
  opts: Opts,
): Promise<{ status: number; json: unknown }> {
  const headers: Record<string, string> = { Authorization: `Bearer ${token()}` };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const res = await globalThis.fetch(ROOT + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: opts.signal,
  });
  const text = await res.text();
  let json: unknown = null;
  if (text) {
    try {
      json = JSON.parse(text);
    } catch {
      json = null;
    }
  }
  return { status: res.status, json };
}

function fail(status: number, json: unknown): ApiError {
  const o = (json ?? {}) as { error?: unknown; diagnostic?: Diagnostic };
  const message = typeof o.error === "string" ? o.error : `request failed (${status})`;
  return new ApiError(status, message, o.diagnostic, json);
}

async function call<T>(method: string, path: string, body: unknown, opts: Opts): Promise<T> {
  const { status, json } = await send(method, path, body, opts);
  if (status < 200 || status > 299) throw fail(status, json);
  return json as T;
}

function query(q: object): string {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(q)) if (v !== undefined) p.set(k, String(v));
  const s = p.toString();
  return s ? `?${s}` : "";
}

export const api = {
  /** Stops the live generator (the engine keeps serving and draining). */
  livePause: (o: Opts = {}) => call<unknown>("POST", "/live/pause", undefined, o),
  liveResume: (o: Opts = {}) => call<unknown>("POST", "/live/resume", undefined, o),
  /** The offered rate of the live run, requests a second (at most 20,000). */
  liveRate: (rps: number, o: Opts = {}) => call<unknown>("PUT", "/live/rate", { rps }, o),
  getDraft: (o: Opts = {}) => call<DraftView>("GET", "/draft", undefined, o),
  putDraft: (body: PutBody, o: Opts = {}) => call<DraftView>("PUT", "/draft", body, o),
  validate: (text?: string, o: Opts = {}) =>
    call<ValidateResult>("POST", "/draft/validate", text === undefined ? undefined : { text }, o),
  apply: (timeoutMs?: number, o: Opts = {}) =>
    call<ApplyResult>(
      "POST",
      "/draft/apply",
      timeoutMs === undefined ? undefined : { timeout_ms: timeoutMs },
      o,
    ),
  save: (body: SaveBody = {}, o: Opts = {}) => call<SaveResult>("POST", "/draft/save", body, o),
  /** Throws the draft away and reads the config file again. */
  reload: (o: Opts = {}) => call<DraftView>("POST", "/draft/reload", undefined, o),
  demo: (o: Opts = {}) => call<{ text: string }>("GET", "/demo", undefined, o),
  /** Sets live chaos (merged into what is there); the body names a structured target. */
  setChaos: (body: Record<string, unknown>, o: Opts = {}) =>
    call<Record<string, unknown>>("PUT", "/chaos", body, o),
  /** The live chaos; `effective.nodes` is what requests meet (scenarios and hand-set together). */
  getChaos: (o: Opts = {}) =>
    call<{ effective?: { nodes?: Record<string, unknown> } }>("GET", "/chaos", undefined, o),
  listScenarios: (o: Opts = {}) => call<ScenarioRow[]>("GET", "/scenarios", undefined, o),
  /** Pushes a scenario on the stack; `ttl` (text such as `30s`) deactivates it when the time is up. */
  activateScenario: (name: string, ttl?: string, o: Opts = {}) =>
    call<unknown>(
      "POST",
      `/scenarios/${encodeURIComponent(name)}/activate`,
      ttl === undefined ? undefined : { ttl },
      o,
    ),
  deactivateScenario: (name: string, o: Opts = {}) =>
    call<unknown>("POST", `/scenarios/${encodeURIComponent(name)}/deactivate`, undefined, o),
  clearScenarios: (o: Opts = {}) => call<unknown>("DELETE", "/scenarios", undefined, o),
  getPins: (o: Opts = {}) => call<PinsView>("GET", "/pins", undefined, o),
  putPin: (body: PinBody, o: Opts = {}) => call<LivePin>("PUT", "/pins", body, o),
  /** Steps the endpoint's live pin to the next state (or pins `state`). */
  cyclePin: (body: PinBody, o: Opts = {}) => call<LivePin>("POST", "/cycle", body, o),
  /** Removes one endpoint's live pin, or every live pin with no endpoint. */
  deletePins: (endpoint?: string, o: Opts = {}) =>
    call<unknown>("DELETE", `/pins${query({ endpoint })}`, undefined, o),
  /** One response body the mock would generate for `endpoint` (a seed picks the data). */
  sample: (endpoint: string, seed?: number, o: Opts = {}) =>
    call<SampleResult>(
      "POST",
      "/sample",
      seed === undefined ? { endpoint } : { endpoint, seed },
      o,
    ),
  getTopology: (o: Opts = {}) => call<Topology>("GET", "/topology", undefined, o),
  /** The live response edits (step 8A). */
  getPayloads: (o: Opts = {}) => call<{ edits: PayloadEdit[] }>("GET", "/payload", undefined, o),
  putPayload: (edit: PayloadEdit, o: Opts = {}) => call<null>("PUT", "/payload", edit, o),
  /** Writes the edit into the endpoints file the draft names (it is not part of the draft). */
  stagePayloadFile: (edit: PayloadEdit, o: Opts = {}) =>
    call<{ file: string }>("POST", "/payload/stage-file", edit, o),
  deletePayload: (route: string, o: Opts = {}) =>
    call<null>("DELETE", `/payload${query({ route })}`, undefined, o),
  /** What a route answers with no edit: a starting text for the editor. */
  previewPayload: (route: string, o: Opts = {}) =>
    call<PayloadPreview>("GET", `/payload/preview${query({ route })}`, undefined, o),
  /** The mock's request log: the first rows finished after `after` (completion order, a page at a time), or the newest ones with no cursor. */
  getFeed: (q: { after?: number; limit: number }, o: Opts = {}) =>
    call<{ entries: FeedRow[]; boot?: number }>("GET", `/logs${query(q)}`, undefined, o),
  /** A busy simulator (409, or 429 while draft builds are in progress) is a result, not an error. */
  async startRun(body: RunBody, o: Opts = {}): Promise<StartRun> {
    const { status, json } = await send("POST", "/sim/run", body, o);
    if (status === 202) return { kind: "started", id: (json as { id: string }).id };
    if (status === 409 || status === 429) {
      return { kind: "busy", status, message: fail(status, json).message };
    }
    throw fail(status, json);
  },
  getRun: (id: string, o: Opts = {}) =>
    call<RunRecord>("GET", `/sim/runs/${encodeURIComponent(id)}`, undefined, o),
  getLog: (id: string, q: LogQuery = {}, o: Opts = {}) =>
    call<LogPage>("GET", `/sim/runs/${encodeURIComponent(id)}/log${query(q)}`, undefined, o),
};
