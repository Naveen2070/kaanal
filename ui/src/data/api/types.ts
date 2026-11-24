// Mirrors of the control API's JSON (docs/web/control-api.md). The server owns these shapes; a
// field is optional here only when the server may leave it out or send null.

/** Node name to `[x, y]`. */
import type { LiveEdit } from "./liveTypes";

export type Layout = Record<string, [number, number]>;

/** One node of the draft as the file writes it (`type` plus its own fields). */
export interface NodeDoc {
  type: string;
  [field: string]: unknown;
}

/** One edge of the draft. An end written bare in YAML may be a number or boolean (R1.90): compare `String(from)`. */
export interface EdgeDoc {
  from: string | number | boolean;
  to: string | number | boolean;
  [field: string]: unknown;
}

/** The draft text as JSON: the file's own keys. */
export interface Doc {
  nodes?: Record<string, NodeDoc>;
  edges?: EdgeDoc[];
  [key: string]: unknown;
}

/** A problem placed in a text (`POST /draft/validate`, a draft run, `doc_error`). */
export interface Diagnostic {
  file: string | null;
  line: number | null;
  col: number | null;
  path: (string | number)[];
  message: string;
}

/** One node kind's rules as the server states them. */
export interface KindRules {
  kind: string;
  /** The kinds a node of this kind may call. */
  callees: string[];
  /** True when the kind has one `proc` service time (the others price their work per operation). */
  service_time: boolean;
}

export interface DraftView {
  /** What this change did to the running live system, when a page follows the live run. */
  live?: LiveEdit;
  text: string;
  dirty: boolean;
  generation: number;
  lost: string[];
  /** Counts every change of `text`; restarts at 0 with the server process (pair with `boot`). */
  rev: number;
  boot: string;
  doc: Doc | null;
  doc_error: Diagnostic | null;
  project_id: string;
  /** The config file's folder name (a base name, never a path); null with no file. */
  project_name: string | null;
  /** Per node kind, the node Studio creates. */
  kind_defaults: Record<string, NodeDoc>;
  /**
   * The rules of each node kind, from the server (the engine's own table): who may call whom, and
   * whether the kind has one service time. Absent from an older server; Studio then uses its own copy.
   */
  kinds?: KindRules[];
  layout: Layout;
  layout_note: string | null;
  layout_dirty: boolean;
  /**
   * The canvas notes, kept in the layout sidecar beside the positions. Absent from an older
   * server: Studio then has none to show (and cannot save any).
   */
  canvas_notes?: Note[];
  /** The text equals what Save last read or wrote and the layout is as saved. */
  saved: boolean;
  /** `rename_node` edits made since the text was set, in order: what `fix_scenarios: rewrite` follows. */
  renames?: [string, string][];
}

/** One edit of the draft text (`PUT /draft` with `patch`). */
export type Op =
  | { op: "set"; path: (string | number)[]; value: unknown }
  | { op: "insert"; path: (string | number)[]; value: unknown }
  /** Adds `value` as the last item of the list at `path`, in the list's own style. */
  | { op: "append"; path: (string | number)[]; value: unknown }
  | { op: "remove"; path: (string | number)[] }
  /** Exchanges two items of one list; each keeps its own text and comments. */
  | { op: "swap"; path: (string | number)[]; with: (string | number)[] }
  | { op: "add_node"; name: string; kind: string }
  | { op: "rename_node"; from: string; to: string };

/** A caption pinned to the canvas. */
export interface Note {
  id: string;
  text: string;
  /** Top-left corner, in flow coordinates. */
  at: [number, number];
}

export type PutBody =
  | { text: string }
  | { patch: Op[]; layout?: Layout }
  | { layout: Layout }
  | { canvas_notes: Note[] };

export interface ValidateResult {
  ok: boolean;
  diagnostics: Diagnostic[];
  warnings: string[];
  flagged: { file: string; scenario: string; message: string }[];
  /** Route key to the name of the component that owns it (empty when the draft does not build). */
  owners?: Record<string, string>;
}

export interface ApplyResult {
  ok: boolean;
  generation: number;
  kept: Record<string, unknown>;
  dropped: Record<string, string[]>;
  flagged: ValidateResult["flagged"];
}

export interface SaveBody {
  confirm_loss?: boolean;
  fix_scenarios?: "none" | "remove" | "rewrite";
  /** Write even though the file changed on disk since Studio read it. */
  overwrite?: boolean;
}

export interface SaveResult {
  ok: boolean;
  written: string[];
  removed: string[];
  layout_written: boolean;
  layout_note: string | null;
}

export interface Topology {
  nodes: { name: string; type: string; flavor?: string }[];
  edges: { from: string; to: string }[];
  routes: string[];
  endpoints: { key: string; visits: Record<string, number> }[];
}

export interface RunBody {
  /** A profile name from `traffic:`, or a profile written out. */
  profile: string | Record<string, unknown>;
  duration_s?: number;
  warmup_s?: number;
  seed?: number;
  scenario?: string;
  source?: "applied" | "draft";
}

export interface Bucket {
  le_ms: number | null;
  count: number;
}

export interface Series {
  window_s: number;
  busy: number[];
  queue: number[];
  /** Per window, counts in the summary latency buckets. */
  latency: number[][];
}

export interface SystemSeries {
  window_s: number;
  /** Requests finished OK per window (60). */
  done: number[];
  /** Requests finished failed per window. */
  failed: number[];
  /** End-to-end p99 per window, a bucket upper bound; 0 for an empty window. */
  p99_ms: number[];
}

export interface ReplicaSummary {
  replica: number;
  visits: number;
  visits_per_s: number;
  mean_wait_ms: number;
  offered_load: number | null;
  utilization: number | null;
}

export interface NodeSummary {
  name: string;
  /** Requests a second that failed here by chaos or a pinned error (Live only; absent in a batch run). */
  failed_per_s?: number;
  replicas: ReplicaSummary[];
  visits_per_s: number;
  expected_visits_per_s: number;
  mean_wait_ms: number;
  offered_load: number | null;
  utilization: number | null;
  saturated: boolean;
  max_queue: number;
  capacity_per_s: number | null;
  /** Requests per second the whole mix can reach before this node saturates. */
  saturates_at_req_s: number | null;
  headroom: number | null;
  /** Own work over cores x replicas x window; null without `cpu`. 1 or more: the CPUs cannot keep up (step 8). */
  cpu_load?: number | null;
  /** Visits per second the CPUs can take; null without `cpu`. */
  cpu_capacity_per_s?: number | null;
  /** CPU the work really used over cores x replicas x window, at most 1 (step 8U); null without `cpu`. */
  cpu_utilization?: number | null;
  /** Which limit sets `saturates_at_req_s`: the slots, the memory or the CPUs, whichever is lower. */
  saturates_by?: "slots" | "memory" | "cpu" | null;
  /** Hop work-time percentiles (wait excluded), exact over every hop; null without visits. */
  p50_ms: number | null;
  p99_ms: number | null;
  series: Series;
  limited: number;
  overflowed: number;
  timed_out: number;
  /** Calls an open circuit breaker refused without sending (step 7c). */
  short_circuited: number;
  /** Times a copy was killed for lack of memory (`memory.on_limit: kill`, step 8K). */
  oom_kills?: number;
}

/** A wire over the run, in the same 60 windows as a node's `Series`: time on the wire (transfer plus time lost sharing it). */
export type LinkSeries = Pick<Series, "window_s" | "latency">;

/** One wire that has a bandwidth. */
export interface LinkSummary {
  from: string;
  to: string;
  transfers: number;
  mean_wait_ms: number;
  offered_load: number;
  utilization: number;
  saturated: boolean;
  /** Requests per second the whole mix can reach before this wire saturates; null when nothing crossed it. */
  saturates_at_req_s: number | null;
  series: LinkSeries;
}

/** One side of a node's network card (`bandwidth:` on a node). */
export interface PipeSummary {
  node: string;
  direction: "send" | "receive";
  transfers: number;
  mean_wait_ms: number;
  offered_load: number;
  utilization: number;
  saturated: boolean;
  /** Requests per second the whole mix can reach before this pipe saturates; null when nothing crossed it. */
  saturates_at_req_s: number | null;
}

export interface Summary {
  label: string;
  window_s: number;
  requests: number;
  failed: number;
  errors: Record<string, number>;
  warnings: string[];
  latency: { p50_ms: number; p95_ms: number; p99_ms: number; max_ms: number; buckets: Bucket[] };
  series: SystemSeries;
  streams: { endpoint: string; requests: number; failed: number; [k: string]: unknown }[];
  nodes: NodeSummary[];
  links: LinkSummary[];
  /** Node network pipes with a rate; absent from an older server. */
  pipes?: PipeSummary[];
  /** Indexes into `nodes`. */
  bottlenecks: number[];
  /** Per wire that carried a retry scope: how its calls ended; absent from an older server. */
  retries?: RetrySummary[];
  /** Per circuit breaker that changed state: its timeline; absent from an older server. */
  breakers?: BreakerSummary[];
}

/** What the retry scopes of one wire did over the run. */
export interface RetrySummary {
  from: string;
  to: string;
  /** Calls that carried a retry scope (the first tries). */
  calls: number;
  /** Calls that made at least one retry. */
  retried: number;
  /** Retry attempts, all calls. */
  retries: number;
  /** Retried calls that ended well. */
  recovered: number;
  /** Retried calls that did not (`retried - recovered`). */
  gave_up: number;
  /** Calls whose last attempt failed, retried or not. */
  failed: number;
}

export type BreakerPhase = "closed" | "half_open" | "open";

/** One breaker (a wire and the caller replica holding it) over the run. */
export interface BreakerSummary {
  from: string;
  to: string;
  replica: number;
  /** The state when the reporting window began. */
  state_at_start: BreakerPhase;
  /** State changes inside the window, `t_ms` after it began. */
  changes: { t_ms: number; state: BreakerPhase }[];
  /** Milliseconds spent open inside the window. */
  open_ms: number;
}

export interface RunRecord {
  id: string;
  /** `run` or `sweep`; the server always sends it, optional only for an older server. */
  kind?: "run" | "sweep";
  state: "running" | "done" | "failed";
  generation: number;
  profile: string;
  scenario: string | null;
  source: "applied" | "draft";
  /** The draft's `rev` a draft run was built from. */
  draft_rev: number | null;
  started_ms: number;
  finished_ms: number | null;
  summary?: Summary | null;
  error?: string | null;
}

/** The body of `POST /sim/sweep`. `error_pct` is a percentage (0 to 100); the record carries it as a fraction. */
export interface SweepBody {
  /** A profile name from `traffic:`, or a profile written out; none sweeps the applied design's first. */
  profile?: string | object;
  seed?: number;
  scenario?: string;
  /** Highest p99 that passes, ms. */
  p99_ms?: number;
  error_pct?: number;
  /** Highest load factor on the ladder, 1 to 64. */
  max_factor?: number;
}

/** One node's load at one sweep point. */
export interface SweepNodeDoc {
  name: string;
  utilization: number | null;
  saturated: boolean;
  saturates_at_req_s: number | null;
}

/** One load factor of a sweep. */
export interface SweepPointDoc {
  k: number;
  req_s: number;
  requests: number;
  goodput_req_s: number;
  /** A share, 0 to 1. */
  error_rate: number;
  p50_ms: number;
  p95_ms: number;
  p99_ms: number;
  cause: string | null;
  nodes: SweepNodeDoc[];
  over: { p99: boolean; errors: boolean };
}

export type SweepVerdict = {
  kind: "knee" | "broken_at_lowest" | "holds_to";
  k: number;
  req_s: number;
};

/** What a finished sweep found. */
export interface SweepResultDoc {
  label: string;
  limit: { p99_ms: number; error_rate: number };
  points: SweepPointDoc[];
  verdict: SweepVerdict;
  recovers_at: number | null;
  stopped_at: number | null;
  first_to_saturate: {
    node: string;
    k: number;
    req_s: number;
    analytic_req_s: number | null;
  } | null;
  dominant_cause: string | null;
  warnings: string[];
}

/** The live chaos a sweep ran under: active scenarios (bottom to top), the live entry count, a 12-hex digest of the entries. */
export interface LiveChaos {
  scenarios: string[];
  entries: number;
  digest: string;
}

/** A sweep's record from `GET /sim/runs/{id}`. */
export interface SweepRecord {
  id: string;
  kind: "sweep";
  state: "running" | "done" | "failed";
  generation: number;
  profile: string;
  seed: number;
  max_factor: number;
  fingerprint: string;
  /** The scenario whose chaos alone the sweep ran under; absent or null for the live chaos. */
  scenario?: string | null;
  /** Without a `scenario`: the live chaos in force when the sweep started (the fingerprint covers the design files only). */
  live_chaos?: LiveChaos;
  /** 12 hex digits over the compiled chaos the sweep ran under; with the fingerprint and the seed it says whether two sweeps match. */
  chaos_digest?: string;
  /** Scenarios in force defined through the control API: the re-run command alone cannot repeat them. */
  runtime_scenarios?: string[];
  /** Points in run order while it runs; kept when it fails. */
  sweep_points: SweepPointDoc[];
  /** The finding, when `done`. */
  sweep?: SweepResultDoc | null;
  error?: string | null;
}

export interface LogLine {
  t_ms: number;
  level: "info" | "warn" | "error";
  request: number;
  stream: string;
  node: string;
  /** On a wire hop (`link request`, `link response`): the node that called over it; the wire is `from` to `node`. */
  from?: string;
  /** Null on a failure line. */
  replica: number | null;
  /** `retry` is a second or later attempt of a call starting: no time of its own, not a failure. */
  event:
    | "served"
    | "queued"
    | "failed"
    | "limited"
    | "overflowed"
    | "timeout"
    | "short-circuited"
    | "retry";
  wait_ms: number | null;
  service_ms: number | null;
  status: number | null;
  note: string | null;
  /** On a hop that ran in a retry: which try (2 or more); absent in a first attempt. */
  attempt?: number;
}

export interface LogQuery {
  node?: string;
  /** With `to`: only the hops on the wire from one node to another. */
  from?: string;
  to?: string;
  replica?: number;
  endpoint?: string;
  level?: "info" | "warn" | "error";
  request?: number;
  limit?: number;
  after?: number;
}

export interface LogPage {
  lines: LogLine[];
  next: number | null;
  total: number;
  sampled: true;
  note: string;
}

/** One row of `GET /scenarios`: a named list of chaos entries and pins. */
/** What `POST /scenarios/save` answers. `warnings` counts leased live fields the file makes permanent. */
export interface SaveScenarioResult {
  name: string;
  path: string;
  warnings: string[];
}

export interface ScenarioRow {
  name: string;
  /** `inline` (kaanal.yaml), `file` (scenarios/NAME.yaml) or `runtime` (defined through the API). */
  source: "inline" | "file" | "runtime";
  description?: string | null;
  /** Chaos entries and pins together. */
  entries: number;
  pins?: { endpoint: string; state: string; status?: number; delay?: string }[];
  active: boolean;
  expires_in_ms?: number;
}

export const PIN_STATES = ["loading", "empty", "error", "slow", "success"] as const;
export type PinState = (typeof PIN_STATES)[number];

/** A live pin: an endpoint forced to answer in one state. */
export interface LivePin {
  endpoint: string;
  state: PinState;
  status?: number;
  delay_ms?: number;
}

/** `GET /pins`: the live pins and the winning pin per endpoint. */
export interface PinsView {
  live: LivePin[];
  effective: Record<string, LivePin & { source: "live" | { scenario: string } }>;
}

/** The body of `PUT /pins` and `POST /cycle`. */
export interface PinBody {
  endpoint: string;
  state?: PinState;
  status?: number;
  /** Text such as `"500ms"`. */
  delay?: string;
}

/** `POST /sample`: one response body the mock would generate for an endpoint. */
export interface SampleResult {
  endpoint: string;
  status: number;
  /** Null for a route with no response schema. */
  body: unknown;
  bytes: number;
  label: string;
}
