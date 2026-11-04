import { expect, test } from "vite-plus/test";
import type { Doc, Op } from "../../data/api/types";
import { applyOps } from "../../data/store/optimistic";
import {
  activeProfile,
  blankRow,
  buildOps,
  gatewayOf,
  keyOf,
  mixOf,
  newConnections,
  problems,
  routeOps,
  rowsFrom,
  serviceNames,
  setWeight,
  targetsOfChain,
  type Row,
} from "./model";

const doc = (over: Partial<Doc> = {}): Doc => ({
  nodes: {
    gw: { type: "gateway" },
    shop: { type: "service" },
    search: { type: "service" },
    legacy: { type: "monolith" },
    db: { type: "database" },
  },
  edges: [{ from: "gw", to: "shop" }],
  endpoints: {
    "GET /products": { op: "read", override: { status: 200 }, chain: [{ call: "shop" }] },
    "POST /orders": { chain: [{ call: "shop" }, { cache: "c" }] }, // written by hand
  },
  traffic: {
    peak: {
      rps: 300,
      journeys: [
        { endpoint: "GET /products", weight: 4, resp_bytes: 2048 },
        { endpoint: "POST /orders", weight: 1, req_bytes: 4096 },
      ],
    },
  },
  ...over,
});

/** `ops` applied to `d`, the way the draft will see them. */
const after = (d: Doc, ops: Op[]): Doc =>
  applyOps({ doc: d, layout: {}, kind_defaults: {} } as never, ops).doc as Doc;

const row = (over: Partial<Row>): Row => ({ ...blankRow(), ...over });

test("the_slider_profile_is_the_first_and_a_blank_draft_gets_peak", () => {
  expect(activeProfile(doc())).toBe("peak");
  expect(activeProfile({ traffic: { night: {}, peak: {} } })).toBe("night");
  expect(activeProfile({})).toBe("peak");
  expect(activeProfile(null)).toBe("peak");
});

test("the_gateway_and_the_services_it_could_reach", () => {
  expect(gatewayOf(doc())).toBe("gw");
  expect(gatewayOf({ nodes: { a: { type: "service" } } })).toBeNull();
  expect(serviceNames(doc())).toEqual(["shop", "search", "legacy"]); // not the database
});

test("a_chain_of_calls_reads_as_services_and_anything_else_is_custom", () => {
  expect(targetsOfChain(undefined)).toEqual([]);
  expect(targetsOfChain([])).toEqual([]);
  expect(targetsOfChain([{ call: "a" }])).toEqual(["a"]);
  expect(targetsOfChain([{ parallel: [{ call: "a" }, { call: "b" }] }])).toEqual(["a", "b"]);
  expect(targetsOfChain([{ call: "a" }, { call: "b" }])).toEqual(["a", "b"]);
  expect(targetsOfChain([{ call: "a", chain: [{ call: "b" }] }])).toEqual(["a"]); // what it does inside is not the builder's
  expect(targetsOfChain([{ cache: "c" }])).toBeNull();
  expect(targetsOfChain("not a list")).toBeNull();
});

test("a_service_that_stays_keeps_its_inner_chain_and_a_new_one_gets_a_plain_call", () => {
  const d = doc({
    endpoints: {
      "GET /products": {
        chain: [{ call: "shop", repeat: 2, chain: [{ call: "db", op: "read" }] }],
      },
    },
  });
  const rows = rowsFrom(d, "peak").slice(0, 1);
  expect(rows[0]?.targets).toEqual(["shop"]);
  const keep = after(d, buildOps(d, "peak", rows)).endpoints as Record<string, { chain: unknown }>;
  expect(keep["GET /products"]?.chain).toEqual([
    { call: "shop", repeat: 2, chain: [{ call: "db", op: "read" }] },
  ]);
  const more = after(
    d,
    buildOps(d, "peak", [{ ...(rows[0] as Row), targets: ["shop", "search"] }]),
  );
  expect((more.endpoints as Record<string, { chain: unknown }>)["GET /products"]?.chain).toEqual([
    {
      parallel: [
        { call: "shop", repeat: 2, chain: [{ call: "db", op: "read" }] },
        { call: "search" },
      ],
    },
  ]);
});

test("rows_come_from_the_journeys_with_sizes_in_kb_and_the_endpoints_chain", () => {
  expect(rowsFrom(doc(), "peak")).toEqual([
    {
      from: "GET /products",
      method: "GET",
      path: "/products",
      weight: 4,
      reqKb: 0,
      respKb: 2,
      targets: ["shop"],
    },
    {
      from: "POST /orders",
      method: "POST",
      path: "/orders",
      weight: 1,
      reqKb: 4,
      respKb: 0,
      targets: null,
    },
  ]);
  expect(rowsFrom(doc(), "nope")).toEqual([]);
  expect(rowsFrom({}, "peak")).toEqual([]);
});

test("applying_unchanged_rows_changes_no_meaning_and_keeps_a_hand_written_chain", () => {
  const d = doc();
  const next = after(d, buildOps(d, "peak", rowsFrom(d, "peak")));
  const e = next.endpoints as Record<string, { chain?: unknown }>;
  expect(e["POST /orders"]?.chain).toEqual([{ call: "shop" }, { cache: "c" }]);
  expect(rowsFrom(next, "peak")).toEqual(rowsFrom(d, "peak"));
});

test("several_services_are_called_in_parallel_and_one_is_a_plain_call", () => {
  const d = doc();
  const rows = [
    row({ path: "/search", targets: ["shop", "search"] }),
    row({ method: "POST", path: "/pay", targets: ["legacy"] }),
  ];
  const next = after(d, buildOps(d, "peak", rows));
  const e = next.endpoints as Record<
    string,
    { chain?: unknown; op?: string; override?: { status: number } }
  >;
  expect(e["GET /search"]?.chain).toEqual([{ parallel: [{ call: "shop" }, { call: "search" }] }]);
  expect(e["GET /search"]?.op).toBe("read");
  expect(e["POST /pay"]?.chain).toEqual([{ call: "legacy" }]);
  expect(e["POST /pay"]?.op).toBe("write");
  expect(e["POST /pay"]?.override?.status).toBe(201);
});

test("a_service_the_gateway_has_no_edge_to_gets_one_and_only_once", () => {
  const d = doc();
  const rows = [
    row({ path: "/a", targets: ["shop", "search"] }),
    row({ path: "/b", targets: ["search"] }),
  ];
  expect(newConnections(d, rows)).toEqual(["search"]);
  const next = after(d, buildOps(d, "peak", rows));
  expect(next.edges).toEqual([
    { from: "gw", to: "shop" },
    { from: "gw", to: "search" },
  ]);
});

test("edges_are_created_when_the_draft_has_none", () => {
  const d = doc();
  delete d.edges;
  const next = after(d, buildOps(d, "peak", [row({ path: "/a", targets: ["shop", "search"] })]));
  expect(next.edges).toEqual([
    { from: "gw", to: "shop" },
    { from: "gw", to: "search" },
  ]);
});

test("sizes_become_bytes_and_zero_means_none", () => {
  const d = doc();
  const rows = [row({ path: "/up", method: "PUT", reqKb: 1.5, respKb: 0, weight: 3 })];
  const next = after(d, buildOps(d, "peak", rows));
  const t = next.traffic as Record<string, { rps: number; journeys: unknown[] }>;
  expect(t.peak?.journeys).toEqual([{ endpoint: "PUT /up", weight: 3, req_bytes: 1536 }]);
  expect(t.peak?.rps).toBe(300); // the slider's rate is left alone
});

test("a_blank_draft_gets_the_profile_and_the_default_rate", () => {
  const d: Doc = { nodes: { gw: { type: "gateway" }, shop: { type: "service" } } };
  const next = after(d, buildOps(d, "peak", [row({ path: "/", targets: ["shop"] })]));
  expect((next.traffic as Record<string, { rps: number }>).peak?.rps).toBe(100);
  expect(Object.keys(next.endpoints as object)).toEqual(["GET /"]);
  expect(next.edges).toEqual([{ from: "gw", to: "shop" }]);
});

test("a_second_profile_is_added_beside_the_first_not_over_it", () => {
  const d = doc();
  const next = after(d, buildOps(d, "night", [row({ path: "/", targets: [] })]));
  expect(Object.keys(next.traffic as object)).toEqual(["peak", "night"]);
});

test("a_renamed_row_carries_the_old_chain_and_reply_over", () => {
  const d = doc();
  const rows = rowsFrom(d, "peak")
    .slice(0, 1)
    .map((r) => ({ ...r, path: "/items" }));
  const next = after(d, buildOps(d, "peak", rows));
  const e = next.endpoints as Record<string, { chain?: unknown }>;
  expect(e["GET /items"]?.chain).toEqual([{ call: "shop" }]);
  expect(e["GET /products"]).toBeDefined(); // the old route stays: it is still a route
  expect(keyOf("GET", "/items")).toBe("GET /items");
});

test("a_row_with_no_services_leaves_the_gateway_to_answer", () => {
  const d = doc();
  const next = after(d, buildOps(d, "peak", [row({ path: "/health", targets: [] })]));
  const e = next.endpoints as Record<string, { chain?: unknown }>;
  expect(e["GET /health"]?.chain).toBeUndefined();
});

test("a_stale_login_entry_is_dropped_so_auth_keeps_its_built_in_route", () => {
  const d = doc({
    nodes: { gw: { type: "gateway" }, auth: { type: "auth" }, shop: { type: "service" } },
    endpoints: {
      "GET /products": { chain: [{ call: "shop" }] },
      "POST /login": { chain: [{ call: "shop" }] }, // would fail: auth has no edge to shop
    },
  });
  const rows = [row({ path: "/products", targets: ["shop"] })]; // the login row was removed
  const e = after(d, buildOps(d, "peak", rows)).endpoints as Record<string, unknown>;
  expect(Object.keys(e)).toEqual(["GET /products"]);
  // a row that still sends it, or another profile that uses it, keeps it
  const keep = after(d, buildOps(d, "peak", [row({ method: "POST", path: "/login" })]));
  expect(Object.keys(keep.endpoints as object)).toContain("POST /login");
  const other = doc({
    ...d,
    traffic: { peak: { journeys: [] }, night: { journeys: [{ endpoint: "POST /login" }] } },
  });
  expect(Object.keys(after(other, buildOps(other, "peak", rows)).endpoints as object)).toContain(
    "POST /login",
  );
  // and without an auth component nothing special happens
  const plain = doc({ endpoints: { "POST /login": { chain: [{ call: "shop" }] } } });
  expect(Object.keys(after(plain, buildOps(plain, "peak", rows)).endpoints as object)).toContain(
    "POST /login",
  );
});

test("the_login_route_cannot_be_sent_through_a_service", () => {
  const d = doc({
    nodes: { gw: { type: "gateway" }, auth: { type: "auth" }, shop: { type: "service" } },
  });
  const login = row({ method: "POST", path: "/login", targets: ["shop"] });
  expect(problems([login], d)[0]).toMatch(/answered by the auth component/);
  expect(problems([{ ...login, targets: [] }], d)).toEqual([]);
  expect(problems([login], doc())).toEqual([]); // no auth component: an ordinary route
});

test("what_cannot_be_applied_is_said_in_words", () => {
  const d = doc();
  expect(problems([], d)).toEqual(["Add at least one endpoint."]);
  expect(problems([row({ path: "nope" })], d)[0]).toMatch(/starts with \//);
  expect(problems([row({ path: "/a b" })], d)[0]).toMatch(/no spaces/);
  expect(problems([row({ path: "/a", weight: 0 })], d)[0]).toMatch(/weight/);
  expect(problems([row({ path: "/a", reqKb: -1 })], d)[0]).toMatch(/request size/);
  expect(problems([row({ path: "/a", respKb: 2_000_000 })], d)[0]).toMatch(/response size/);
  const dup = problems([row({ path: "/a" }), row({ path: "/A" }), row({ path: "/a" })], d);
  expect(dup.join(" ")).toMatch(/repeats endpoint 1/);
  const noGateway = { nodes: { shop: { type: "service" } } };
  expect(problems([row({ path: "/a", targets: ["shop"] })], noGateway)[0]).toMatch(/Add a gateway/);
  expect(problems([row({ path: "/a" })], doc({ endpoints: "kaanal.endpoints.yaml" }))[0]).toMatch(
    /separate file/,
  );
  expect(problems([row({ path: "/a", targets: ["shop"] })], d)).toEqual([]);
});

test("an_endpoints_route_is_set_to_one_service_several_in_parallel_or_none", () => {
  const d = doc();
  const set = (targets: string[]) =>
    (
      after(d, routeOps(d, "GET /products", targets)).endpoints as Record<
        string,
        { chain?: unknown }
      >
    )["GET /products"]?.chain;
  expect(set(["shop"])).toEqual([{ call: "shop" }]);
  expect(set(["shop", "search"])).toEqual([{ parallel: [{ call: "shop" }, { call: "search" }] }]);
  expect(set([])).toBeUndefined();
  // the gateway is wired to a service it had no wire to
  expect(after(d, routeOps(d, "GET /products", ["shop", "search"])).edges).toEqual([
    { from: "gw", to: "shop" },
    { from: "gw", to: "search" },
  ]);
  // a chain written by hand is not touched
  expect(routeOps(d, "POST /orders", ["shop"])).toEqual([]);
});

test("the_mix_is_each_journeys_share_of_the_whole_and_what_that_is_per_second", () => {
  const d = doc();
  const mix = mixOf(d, "peak");
  expect(mix.map((r) => [r.key, r.index, r.weight])).toEqual([
    ["GET /products", 0, 4],
    ["POST /orders", 1, 1],
  ]);
  expect(mix.map((r) => r.share)).toEqual([0.8, 0.2]);
  expect(mix.map((r) => r.perS)).toEqual([240, 60]); // 300 req/s in all
  // a journey with no weight counts as 1; a profile with no rate has no per-second figure
  const bare = doc({
    traffic: { peak: { journeys: [{ endpoint: "GET /a" }, { endpoint: "GET /b" }] } },
  });
  expect(mixOf(bare, "peak").map((r) => [r.share, r.perS])).toEqual([
    [0.5, null],
    [0.5, null],
  ]);
  expect(mixOf(d, "nope")).toEqual([]);
  expect(mixOf(null, "peak")).toEqual([]);
});

test("a_new_weight_is_one_patch_at_the_journeys_place", () => {
  const d = doc();
  const next = after(d, [setWeight("peak", 1, 3)]);
  expect(mixOf(next, "peak").map((r) => r.share)).toEqual([4 / 7, 3 / 7]);
  expect(setWeight("peak", 1, 3)).toEqual({
    op: "set",
    path: ["traffic", "peak", "journeys", 1, "weight"],
    value: 3,
  });
});

test("sizes_written_with_a_unit_are_read_and_stay_as_written_until_the_row_changes", () => {
  const d = doc({
    traffic: {
      peak: {
        rps: 300,
        journeys: [
          { endpoint: "GET /products", weight: 4, resp_bytes: "2KB" },
          { endpoint: "POST /orders", weight: 1, req_bytes: "1.5 KB" },
        ],
      },
    },
  });
  const rows = rowsFrom(d, "peak");
  expect(rows.map((r) => [r.reqKb, r.respKb])).toEqual([
    [0, 2],
    [1.5, 0],
  ]);
  const journeys = (x: Doc) =>
    (x.traffic as never as Record<string, { journeys: unknown[] }>).peak?.journeys;
  // applying the rows as they are keeps the text
  expect(journeys(after(d, buildOps(d, "peak", rows)))).toEqual(journeys(d));
  // a row the person changed is written as bytes, as before
  const edited = rows.map((r, i) => (i === 0 ? { ...r, respKb: 3 } : r));
  expect(journeys(after(d, buildOps(d, "peak", edited)))).toEqual([
    { endpoint: "GET /products", weight: 4, resp_bytes: 3072 },
    { endpoint: "POST /orders", weight: 1, req_bytes: "1.5 KB" },
  ]);
  // text the page cannot read is an empty box
  const odd = doc({
    traffic: { peak: { rps: 1, journeys: [{ endpoint: "GET /a", req_bytes: "big" }] } },
  });
  expect(rowsFrom(odd, "peak")[0]?.reqKb).toBe(0);
});
