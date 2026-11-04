// Starter systems for an empty canvas: each is a whole kaanal.yaml, loaded into the draft through the
// draft store (so it lands after any edit still on its way). The shop is the server's own demo; the
// other two are written here. Each says in a line what to try, since a starter is for learning what
// the numbers do.

import { api } from "../data/api/client";

export interface Starter {
  id: string;
  title: string;
  blurb: string;
  /** What to try once it is on screen. */
  tryThis: string;
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

export const STARTERS: Starter[] = [
  {
    id: "shop",
    title: "A shop",
    blurb: "A gateway, two services, a cache and a database",
    tryThis: "Raise the offered load and watch which part breaks first.",
    text: async () => (await api.demo()).text,
  },
  {
    id: "simple",
    title: "One app and a database",
    blurb: "The smallest useful system",
    tryThis:
      "Lower the app's slots to 1: the queue builds up to 100, then requests are turned away.",
    text: async () => SIMPLE,
  },
  {
    id: "fan-out",
    title: "A fan-out",
    blurb: "One request calls three services that share a database",
    tryThis: "Cut the database pool to 2: everything slows together.",
    text: async () => FAN_OUT,
  },
  {
    id: "balanced",
    title: "A balanced tier",
    blurb: "A gateway, a load balancer, three copies of an API and a database",
    tryThis:
      "Change the API's Balance from power of two choices to round robin, and slow one copy on the Live tab: compare the p99.",
    text: async () => BALANCED,
  },
  {
    id: "file-store",
    title: "A file store",
    blurb: "A gateway, an app and an object store (S3-like) that serves 5 MB files",
    tryThis:
      "Change the reply size of the journey, or the store's Throughput: the time of a call is the first byte plus the transfer. The store's numbers are estimates.",
    text: async () => FILE_STORE,
  },
];
