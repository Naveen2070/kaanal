// Starter systems for an empty canvas: each is a whole kaanal.yaml, loaded into the draft through the
// draft store (so it lands after any edit still on its way). The shop is the server's own demo; the
// other two are written here. Each says in a line what to try, since a starter is for learning what
// the numbers do.

import { api } from "../data/api/client";
import type { StarterGraph } from "./sketch";

export interface Starter {
  id: string;
  title: string;
  blurb: string;
  /** What to try once it is on screen. */
  tryThis: string;
  /** The components and wires of the system, for the picture on its card (see `sketch.ts`). */
  graph: StarterGraph;
  text: () => Promise<string>;
}

const SIMPLE = `version: 1
seed: 7
nodes:
  gateway: { type: gateway, concurrency: { limit: 256, queue: 100 }, proc: { p50: 1ms, p99: 3ms }, timeout: 30s }
  app: { type: monolith, concurrency: { limit: 16, queue: 100 }, proc: { p50: 12ms, p99: 40ms } }
  db: { type: database, flavor: postgres, pool: { size: 10 } }
edges:
  - { from: gateway, to: app, net: { p50: 1ms, p99: 3ms } }
  - { from: app, to: db, net: { p50: 1ms, p99: 3ms } }
endpoints:
  "GET /items":
    op: read
    override: { body: { items: [] } }
    chain: [{ call: app, chain: [{ call: db, op: read }] }]
traffic:
  peak:
    rps: 100
    duration: 30s
    journeys:
      - { endpoint: "GET /items", weight: 1 }
`;

const FAN_OUT = `version: 1
seed: 7
nodes:
  gateway: { type: gateway, concurrency: { limit: 256, queue: 100 }, proc: { p50: 1ms, p99: 3ms }, timeout: 30s }
  web: { type: service, concurrency: { limit: 16, queue: 100 }, proc: { p50: 4ms, p99: 12ms } }
  users: { type: service, concurrency: { limit: 8, queue: 100 }, proc: { p50: 6ms, p99: 20ms } }
  orders: { type: service, concurrency: { limit: 8, queue: 100 }, proc: { p50: 9ms, p99: 30ms } }
  catalog: { type: service, concurrency: { limit: 8, queue: 100 }, proc: { p50: 5ms, p99: 18ms } }
  db: { type: database, flavor: postgres, pool: { size: 20 } }
edges:
  - { from: gateway, to: web, net: { p50: 1ms, p99: 3ms } }
  - { from: web, to: users, net: { p50: 1ms, p99: 3ms } }
  - { from: web, to: orders, net: { p50: 1ms, p99: 3ms } }
  - { from: web, to: catalog, net: { p50: 1ms, p99: 3ms } }
  - { from: users, to: db, net: { p50: 1ms, p99: 3ms } }
  - { from: orders, to: db, net: { p50: 1ms, p99: 3ms } }
  - { from: catalog, to: db, net: { p50: 1ms, p99: 3ms } }
endpoints:
  "GET /home":
    op: read
    override: { body: { ok: true } }
    chain:
      - call: web
        chain:
          - { call: users, chain: [{ call: db, op: read }] }
          - { call: orders, chain: [{ call: db, op: read }] }
          - { call: catalog, chain: [{ call: db, op: read }] }
traffic:
  peak:
    rps: 100
    duration: 30s
    journeys:
      - { endpoint: "GET /home", weight: 1 }
`;

const BALANCED = `version: 1
seed: 7
nodes:
  gateway: { type: gateway, concurrency: { limit: 256, queue: 100 }, proc: { p50: 1ms, p99: 3ms }, timeout: 30s }
  balancer: { type: load-balancer, concurrency: { limit: 512, queue: 100 }, proc: { p50: 200us, p99: 600us } }
  api: { type: service, concurrency: { limit: 8, queue: 100 }, replicas: 3, balance: p2c, proc: { p50: 12ms, p99: 40ms } }
  db: { type: database, flavor: postgres, pool: { size: 20 } }
edges:
  - { from: gateway, to: balancer, net: { p50: 1ms, p99: 3ms } }
  - { from: balancer, to: api, net: { p50: 1ms, p99: 3ms } }
  - { from: api, to: db, net: { p50: 1ms, p99: 3ms } }
endpoints:
  "GET /items":
    op: read
    override: { body: { items: [] } }
    chain: [balancer]
traffic:
  peak:
    rps: 150
    duration: 30s
    journeys:
      - { endpoint: "GET /items", weight: 1 }
`;

const FILE_STORE = `version: 1
seed: 7
nodes:
  gateway: { type: gateway, concurrency: { limit: 256, queue: 100 }, proc: { p50: 1ms, p99: 3ms }, timeout: 30s }
  app: { type: service, concurrency: { limit: 16, queue: 100 }, proc: { p50: 6ms, p99: 20ms } }
  files: { type: storage, flavor: s3, first_byte: 25ms, throughput: 80MB/s }
edges:
  - { from: gateway, to: app, net: { p50: 1ms, p99: 3ms } }
  - { from: app, to: files, net: { p50: 1ms, p99: 3ms } }
endpoints:
  "GET /files":
    op: read
    override: { body: { name: report.pdf } }
    chain: [{ call: app, chain: [{ call: files, op: read }] }]
traffic:
  peak:
    rps: 20
    duration: 30s
    journeys:
      - { endpoint: "GET /files", weight: 1, resp_bytes: 5MB }
`;

const RETRY_STORM = `version: 1
seed: 7
nodes:
  gateway: { type: gateway, concurrency: { limit: 256, queue: 100 }, proc: { p50: 1ms, p99: 3ms }, timeout: 30s }
  app: { type: service, concurrency: { limit: 2, queue: 100 }, proc: { mean: 10ms }, timeout: 50ms }
edges:
  - { from: gateway, to: app, net: { p50: 1ms, p99: 3ms }, retry: { max_attempts: 3 }, abandon: continue }
endpoints:
  "GET /items":
    op: read
    override: { body: { items: [] } }
    chain: [app]
traffic:
  peak:
    rps: 120
    duration: 40s
    journeys:
      - { endpoint: "GET /items", weight: 1 }
`;

export const STARTERS: Starter[] = [
  {
    id: "shop",
    title: "A shop",
    blurb: "A gateway, a shop service, a cache, a database and a payments API",
    tryThis: "Raise the offered load and watch which part breaks first.",
    // the server's demo (crates/kaanal/demo/kaanal.yaml): its text is fetched, so this is written by hand
    graph: {
      nodes: [
        { id: "nginx", kind: "gateway" },
        { id: "shop", kind: "service" },
        { id: "db", kind: "database" },
        { id: "cache", kind: "cache" },
        { id: "payments", kind: "external-api" },
      ],
      wires: [
        ["nginx", "shop"],
        ["shop", "db"],
        ["shop", "cache"],
        ["shop", "payments"],
      ],
    },
    text: async () => (await api.demo()).text,
  },
  {
    id: "simple",
    title: "One app and a database",
    blurb: "The smallest useful system",
    tryThis:
      "Lower the app's slots to 1: the queue builds up to 100, then requests are turned away.",
    graph: {
      nodes: [
        { id: "gateway", kind: "gateway" },
        { id: "app", kind: "monolith" },
        { id: "db", kind: "database" },
      ],
      wires: [
        ["gateway", "app"],
        ["app", "db"],
      ],
    },
    text: async () => SIMPLE,
  },
  {
    id: "fan-out",
    title: "A fan-out",
    blurb: "One request calls three services that share a database",
    tryThis: "Cut the database pool to 2: everything slows together.",
    graph: {
      nodes: [
        { id: "gateway", kind: "gateway" },
        { id: "web", kind: "service" },
        { id: "users", kind: "service" },
        { id: "orders", kind: "service" },
        { id: "catalog", kind: "service" },
        { id: "db", kind: "database" },
      ],
      wires: [
        ["gateway", "web"],
        ["web", "users"],
        ["web", "orders"],
        ["web", "catalog"],
        ["users", "db"],
        ["orders", "db"],
        ["catalog", "db"],
      ],
    },
    text: async () => FAN_OUT,
  },
  {
    id: "balanced",
    title: "A balanced tier",
    blurb: "A gateway, a load balancer, three copies of an API and a database",
    tryThis:
      "Change the API's Balance from power of two choices to round robin, and slow one copy on the Try live tab: compare the p99.",
    graph: {
      nodes: [
        { id: "gateway", kind: "gateway" },
        { id: "balancer", kind: "load-balancer" },
        { id: "api", kind: "service", copies: 3 },
        { id: "db", kind: "database" },
      ],
      wires: [
        ["gateway", "balancer"],
        ["balancer", "api"],
        ["api", "db"],
      ],
    },
    text: async () => BALANCED,
  },
  {
    id: "file-store",
    title: "A file store",
    blurb: "A gateway, an app and an object store (S3-like) that serves 5 MB files",
    tryThis:
      "Change the reply size of the journey, or the store's Throughput: the time of a call is the first byte plus the transfer. The store's numbers are estimates.",
    graph: {
      nodes: [
        { id: "gateway", kind: "gateway" },
        { id: "app", kind: "service" },
        { id: "files", kind: "storage" },
      ],
      wires: [
        ["gateway", "app"],
        ["app", "files"],
      ],
    },
    text: async () => FILE_STORE,
  },
  {
    id: "retry-storm",
    title: "A retry storm",
    blurb: "An app that can take 200 requests a second, retried when it times out",
    tryThis:
      "Run it: at 120 a second, well under what the app can take, most requests still fail, because every timeout is retried and the app keeps working on calls nobody waits for. Then, on the wire, switch on Limit retries with a budget or the circuit breaker, and run again.",
    graph: {
      nodes: [
        { id: "gateway", kind: "gateway" },
        { id: "app", kind: "service" },
      ],
      wires: [["gateway", "app"]],
      failing: [["gateway", "app"]],
    },
    text: async () => RETRY_STORM,
  },
];
