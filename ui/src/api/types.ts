// Mirrors of the control API's JSON (docs/web/control-api.md). The server owns these shapes; a
// field is optional here only when the server may leave it out or send null.

/** Node name to `[x, y]`. */
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

export interface DraftView {
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
  /** Per node kind, the node Studio creates. */
  kind_defaults: Record<string, NodeDoc>;
  layout: Layout;
  layout_note: string | null;
  layout_dirty: boolean;
}

/** One edit of the draft text (`PUT /draft` with `patch`). */
export type Op =
  | { op: "set"; path: (string | number)[]; value: unknown }
  | { op: "insert"; path: (string | number)[]; value: unknown }
  /** Adds `value` as the last item of the list at `path`, in the list's own style. */
  | { op: "append"; path: (string | number)[]; value: unknown }
  | { op: "remove"; path: (string | number)[] }
  | { op: "add_node"; name: string; kind: string }
  | { op: "rename_node"; from: string; to: string };

export type PutBody = { text: string } | { patch: Op[]; layout?: Layout } | { layout: Layout };

export interface ValidateResult {
  ok: boolean;
  diagnostics: Diagnostic[];
  warnings: string[];
  flagged: { file: string; scenario: string; message: string }[];
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
  series: Series;
  limited: number;
  overflowed: number;
  timed_out: number;
}

export interface Summary {
  label: string;
  window_s: number;
  requests: number;
  failed: number;
  errors: Record<string, number>;
  warnings: string[];
  latency: { p50_ms: number; p95_ms: number; p99_ms: number; max_ms: number; buckets: Bucket[] };
  streams: { endpoint: string; requests: number; failed: number; [k: string]: unknown }[];
  nodes: NodeSummary[];
  links: {
    from: string;
    to: string;
    transfers: number;
    mean_wait_ms: number;
    offered_load: number;
    utilization: number;
    saturated: boolean;
  }[];
  /** Indexes into `nodes`. */
  bottlenecks: number[];
}

export interface RunRecord {
  id: string;
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

export interface LogLine {
  t_ms: number;
  level: "info" | "warn" | "error";
  request: number;
  stream: string;
  node: string;
  /** Null on a failure line. */
  replica: number | null;
  event: "served" | "queued" | "failed" | "limited" | "overflowed" | "timeout";
  wait_ms: number | null;
  service_ms: number | null;
  status: number | null;
  note: string | null;
}

export interface LogQuery {
  node?: string;
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
