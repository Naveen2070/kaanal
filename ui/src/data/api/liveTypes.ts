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
  /** Requests to a route whose pin answers an error: the chain ran, the answer is the pin's. */
  pinned?: number;
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
  /** `[running, waiting]` of each copy at the instant of the tick; only for a node with several. */
  copy_load?: [number, number][];
}

/** What the engine counted for a wire's retry scopes in a tick (calls ended in that tick). */
export interface Funnel {
  calls: number;
  retried: number;
  recovered: number;
  failed: number;
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
  /** The engine's retry counts for the wire in the tick; absent without a retry scope. */
  funnel?: Funnel;
  /** How busy the wire was over the tick (1 = full the whole time); absent without a `bandwidth`. */
  load?: number;
}

/** One route of the offered mix in a tick, by index into `hello.streams`. */
export interface EndpointTick {
  requests: number;
  failed: number;
  /** Latency of the served ones: occupied `[bucket, count]` pairs, as `Tick.hist`. */
  hist: [number, number][];
}

/** A circuit breaker (a wire and the caller copy holding it) at the instant of a tick. */
export interface BreakerTick {
  edge: number;
  replica: number;
  state: "closed" | "half_open" | "open";
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
  /** Per route of the offered mix (generated requests); absent when none finished. */
  endpoints?: EndpointTick[];
  /** Every circuit breaker at the instant of the tick; absent when there is none. */
  breakers?: BreakerTick[];
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
