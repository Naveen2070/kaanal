import { expect, test } from "vite-plus/test";
import type { Doc } from "../api/types";
import { cascade, prune } from "./wires";

const doc = (): Doc => ({
  nodes: {
    gw: { type: "gateway" },
    shop: { type: "service" },
    search: { type: "service" },
    db: { type: "database" },
  },
  edges: [
    { from: "gw", to: "shop" },
    { from: "gw", to: "search" },
    { from: "shop", to: "db" },
  ],
  endpoints: {
    "GET /": { chain: [{ call: "shop", chain: [{ call: "db" }] }] },
    "GET /s": { chain: [{ parallel: [{ call: "shop" }, { call: "search" }] }] },
    "GET /x": { chain: [{ call: "search" }] },
    "GET /h": { override: { status: 200 } },
  },
});

test("a_call_over_a_deleted_wire_leaves_every_chain_that_made_it", () => {
  const steps = [{ call: "shop", chain: [{ call: "db" }] }, { call: "search" }];
  expect(prune(steps, "gw", "gw", "search")).toEqual([{ call: "shop", chain: [{ call: "db" }] }]);
  // an inner call is the service's, not the gateway's
  expect(prune(steps, "gw", "shop", "db")).toEqual([
    { call: "shop", chain: [] },
    { call: "search" },
  ]);
  // a call that merely shares the target is not touched when the wire is from another node
  expect(prune([{ call: "db" }], "gw", "shop", "db")).toEqual([{ call: "db" }]);
});

test("deleting_gateway_to_search_edits_the_endpoints_that_used_it", () => {
  const { ops, touched } = cascade(doc(), [1]);
  expect(touched).toEqual(["GET /s", "GET /x"]);
  expect(ops).toEqual([
    {
      op: "set",
      path: ["endpoints", "GET /s", "chain"],
      value: [{ parallel: [{ call: "shop" }] }],
    },
    { op: "remove", path: ["endpoints", "GET /x", "chain"] },
  ]);
});

test("deleting_a_wire_nothing_rides_on_changes_no_endpoint", () => {
  const d = doc();
  d.endpoints = { "GET /": { chain: [{ call: "shop" }] } };
  expect(cascade(d, [1])).toEqual({ ops: [], touched: [] });
  expect(cascade({ nodes: {}, edges: [] }, [0])).toEqual({ ops: [], touched: [] });
});
