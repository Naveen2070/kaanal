import { expect, test } from "vite-plus/test";
import type { Doc, Op } from "../api/types";
import { applyOps } from "../store/optimistic";
import {
  activeProfile,
  blankRow,
  buildOps,
  gatewayOf,
  keyOf,
  newConnections,
  problems,
  rowsFrom,
  serviceNames,
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
