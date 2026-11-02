import { expect, test } from "vite-plus/test";
import type { Doc, EdgeDoc } from "../../data/api/types";
import {
  bandwidthKbOf,
  callsOf,
  clearBandwidth,
  netMsOf,
  setBandwidth,
  setCalls,
  setNet,
  setShare,
  shareApplies,
  shareOf,
} from "./edgeFields";
import { balanceOf, setBalance } from "./fields";

const plain: EdgeDoc = { from: "app", to: "db" };
const full: EdgeDoc = {
  from: "app",
  to: "db",
  net: "4ms",
  calls: 3,
  prob: 0.25,
  bandwidth: 2048,
  balance: "random",
};

test("what_a_wire_says_now_and_the_defaults_when_it_says_nothing", () => {
  expect(netMsOf(plain)).toEqual({ ms: null, spread: false });
  expect(callsOf(plain)).toBe(1);
  expect(shareOf(plain)).toBe(100);
  expect(bandwidthKbOf(plain)).toBeNull();
  expect(netMsOf(full)).toEqual({ ms: 4, spread: false });
  expect(callsOf(full)).toBe(3);
  expect(shareOf(full)).toBe(25);
  expect(bandwidthKbOf(full)).toBe(2);
});

test("a_latency_written_as_a_spread_is_shown_not_edited", () => {
  expect(netMsOf({ ...plain, net: { p50: "4ms", p99: "20ms" } })).toEqual({
    ms: null,
    spread: true,
  });
});

test("each_edit_is_one_patch_at_the_wires_place_set_when_there_insert_when_not", () => {
  expect(setNet(2, plain, 5)).toEqual({ op: "insert", path: ["edges", 2, "net"], value: "5ms" });
  expect(setNet(2, full, 0.5)).toEqual({ op: "set", path: ["edges", 2, "net"], value: "0.5ms" });
  expect(setCalls(0, plain, 4)).toEqual({ op: "insert", path: ["edges", 0, "calls"], value: 4 });
  expect(setShare(0, full, 50)).toEqual({ op: "set", path: ["edges", 0, "prob"], value: 0.5 });
  expect(setShare(0, plain, 0.01)).toEqual({
    op: "insert",
    path: ["edges", 0, "prob"],
    value: 0.0001,
  });
});

test("bandwidth_is_kb_per_second_in_the_box_and_whole_bytes_in_the_file", () => {
  expect(setBandwidth(1, plain, 10)).toEqual({
    op: "insert",
    path: ["edges", 1, "bandwidth"],
    value: 10240,
  });
  // never below one byte per second, never a fraction
  expect(setBandwidth(1, full, 0.0001)).toEqual({
    op: "set",
    path: ["edges", 1, "bandwidth"],
    value: 1,
  });
  expect(setBandwidth(1, full, 1.5)).toMatchObject({ value: 1536 });
  expect(clearBandwidth(1, full)).toEqual([{ op: "remove", path: ["edges", 1, "bandwidth"] }]);
  expect(clearBandwidth(1, plain)).toEqual([]);
});

test("a_share_is_offered_only_where_the_config_accepts_one", () => {
  const doc: Doc = {
    nodes: {
      gw: { type: "gateway" },
      app: { type: "service" },
      db: { type: "database" },
      c: { type: "cache" },
    },
  };
  expect(shareApplies(doc, { from: "app", to: "db" })).toBe(true);
  expect(shareApplies(doc, { from: "gw", to: "app" })).toBe(false); // out of a gateway
  expect(shareApplies(doc, { from: "app", to: "c" })).toBe(false); // into a cache
  expect(shareApplies(null, plain)).toBe(true);
});

test("balance_reads_the_three_shapes_and_the_patch_sets_or_removes_it", () => {
  expect(balanceOf("round-robin")).toBe("round-robin");
  expect(balanceOf("random")).toBe("random");
  expect(balanceOf({ weighted: [3, 1] })).toBe("weighted");
  expect(balanceOf(undefined)).toBeNull();
  expect(balanceOf("nonsense")).toBeNull();
  const path = ["nodes", "app", "balance"];
  expect(setBalance(path, false, "random")).toEqual([{ op: "insert", path, value: "random" }]);
  expect(setBalance(path, true, "round-robin")).toEqual([
    { op: "set", path, value: "round-robin" },
  ]);
  expect(setBalance(path, true, "")).toEqual([{ op: "remove", path }]);
  expect(setBalance(path, false, "")).toEqual([]); // nothing to remove
});
