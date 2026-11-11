// What the live stream (`GET /__kaanal/live`) sends and the draft reply adds (step 8L, RULES 105).
// The shapes are the JSON of `kaanal-server/src/live/tick.rs`: counts and sums, so any window of
// ticks is added up here; nothing is averaged on the server.

export interface Dropped {
  overflow: number;
  rate_limit: number;
  deadline: number;
  breaker: number;
  oom?: number;
  chaos: number;
}

export interface NodeTick {
  /** Own hops that ran here, in requests finished in this tick. */
  arrived: number;
  dropped: Dropped;
  /** Waiting for a slot at the instant of the tick, all copies. */
  queue: number;
  /** Slots in use at that instant, all copies. */
  busy_slots: number;
  /** Running hops over cores x copies; null for a component with no `cpu`. */
  cpu_load: number | null;
  /** The CPU the work really used over the tick's span, at most 1; null without `cpu` or at first. */
  cpu_utilization?: number | null;
  /** The work times of the own hops that ran here: occupied `[bucket, count]` pairs, as `Tick.hist`. */
  hist?: [number, number][];
  /** Own hops that ran at each copy (index = copy); only for a node that ran on more than one. */
  replicas?: number[];
}

export interface WireTick {
  carried: number;
  /** Time spent waiting to get onto the wire, summed over the crossings in the tick (ns). */
  wait_ns?: number;
  /** Time spent on the wire, waiting excluded, summed over the crossings in the tick (ns). */
  wire_ns?: number;
  /** Crossings that were a retry (attempt 2 or later). */
  retries?: number;
  /** Calls that made at least one retry. */
  retried?: number;
}

export interface JourneyHop {
  node: number;
  replica: number;
  kind: string;
  start_ns: number;
  end_ns: number;
  wait_ns: number;
  attempt: number;
  /** The wire a `net` or `link` hop crosses, when known. */
  edge: number | null;
}

/** A finished request drawn as a dot along its hops. */
export interface Journey {
  hops: JourneyHop[];
  /** The index in `hops` where it was dropped; null when it was served. */
  dropped_at: number | null;
  total_ns: number;
  /** The route of a generated request: an index into the session's `streams`. */
  stream?: number;
  /** Why a dropped request was dropped: a key of a node's `dropped`. */
  cause?: string;
  /** A request a real frontend made (step 8A), drawn apart from the generated ones. */
  real?: boolean;
}

export interface TickStatus {
  rate: number;
  paused: boolean;
  overloaded: boolean;
  in_flight: number;
}

/** The real (frontend) requests' share of a tick; the tick's own figures include them. */
export interface RealTick {
  requests: number;
  failed: number;
  nodes: NodeTick[];
  wires: WireTick[];
  hist: [number, number][];
}

export interface Tick {
  /** Milliseconds since the session started, at the end of the tick. */
  t_ms: number;
  /** Milliseconds since the running system started: the clock a `from_ms` outage is set on. */
  system_ms: number;
  requests: number;
  failed: number;
  nodes: NodeTick[];
  wires: WireTick[];
  /** Occupied buckets of the latency histogram of the served requests: `[bucket, count]`. */
  hist: [number, number][];
  journeys: Journey[];
  status: TickStatus;
  /** Only in a tick that had a real request (step 8A). */
  real?: RealTick;
}

export type MarkerKind = "param" | "chaos" | "structure" | "pause" | "resume" | "gap" | "payload";

export interface Marker {
  at_ms: number;
  kind: MarkerKind;
  text: string;
}

export interface LiveHello {
  session: number;
  profile: string | null;
  /** Why nothing is generated (no traffic profile, a route that does not exist), when that is so. */
  note: string | null;
  rate: number;
  paused: boolean;
  overloaded: boolean;
  in_flight: number;
  submitted: number;
  clients: number;
  system_ms: number | null;
  /** The components in the order of every tick's `nodes` (file order; the draft JSON is alphabetical). */
  nodes?: string[];
  /** The routes of the offered mix, in the order a journey's `stream` indexes. */
  streams?: string[];
}

export type LiveEvent =
  | { type: "hello"; hello: LiveHello }
  | { type: "tick"; tick: Tick }
  | { type: "marker"; marker: Marker }
  | { type: "resync"; hello: LiveHello };

/** What a draft change did to the running system (the `live` field of the draft reply). */
export type LiveEdit =
  | { live: "none" }
  | { live: "unchanged"; structure_pending: boolean }
  | { live: "applied"; changed: string[]; structure_pending: boolean }
  | { live: "invalid"; message: string };
