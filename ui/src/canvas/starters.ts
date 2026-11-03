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
  gateway: { type: gateway, concurrency: 256, proc: { p50: 1ms, p99: 3ms } }
  app: { type: monolith, concurrency: 16, proc: { p50: 12ms, p99: 40ms } }
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
  gateway: { type: gateway, concurrency: 256, proc: { p50: 1ms, p99: 3ms } }
  web: { type: service, concurrency: 16, proc: { p50: 4ms, p99: 12ms } }
  users: { type: service, concurrency: 8, proc: { p50: 6ms, p99: 20ms } }
  orders: { type: service, concurrency: 8, proc: { p50: 9ms, p99: 30ms } }
  catalog: { type: service, concurrency: 8, proc: { p50: 5ms, p99: 18ms } }
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
    tryThis: "Lower the app's slots to 2 and see the queue build.",
    text: async () => SIMPLE,
  },
  {
    id: "fan-out",
    title: "A fan-out",
    blurb: "One request calls three services that share a database",
    tryThis: "Cut the database pool to 2: everything slows together.",
    text: async () => FAN_OUT,
  },
];
