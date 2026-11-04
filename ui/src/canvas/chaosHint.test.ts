import { expect, test } from "vite-plus/test";
import { chaosHiddenHint } from "./chaosHint";

const clean = { failed: 0 };

test("down with zero failures hints, naming the node", () => {
  const h = chaosHiddenHint({ db: { down: true } }, clean);
  expect(h).toEqual([
    {
      node: "db",
      text: "Nothing failed although db is down. Requests may never reach it: a cache answers first, or no route calls it. Check Arriving on db.",
    },
  ]);
});

test("down with failures gives nothing", () => {
  expect(chaosHiddenHint({ db: { down: true } }, { failed: 3 })).toEqual([]);
});

test("no chaos, or chaos that does nothing, gives nothing", () => {
  expect(chaosHiddenHint({}, clean)).toEqual([]);
  expect(chaosHiddenHint({ db: { down: false, latency_ms: 0, faults: [] } }, clean)).toEqual([]);
});

test("no finished run gives nothing", () => {
  expect(chaosHiddenHint({ db: { down: true } }, null)).toEqual([]);
  expect(chaosHiddenHint({ db: { down: true } }, undefined)).toEqual([]);
});

test("two nodes with chaos give two hints", () => {
  const h = chaosHiddenHint({ db: { down: true }, cache: { faults: [{ upto: 0.5 }] } }, clean);
  expect(h.map((x) => x.node)).toEqual(["db", "cache"]);
  expect(h[1]?.text).toContain("cache drops some requests");
});

test("slowed wording makes no latency claim", () => {
  const [h] = chaosHiddenHint({ db: { latency_ms: 300 } }, clean);
  expect(h?.text).toBe(
    "db has 300 ms added, but the run shows no failures; if latency looks unchanged, requests may never reach it: a cache answers first, or no route calls it. Check Arriving on db.",
  );
  // down wins over slow
  expect(chaosHiddenHint({ db: { down: true, latency_ms: 300 } }, clean)[0]?.text).toContain(
    "is down",
  );
});

test("a cache that is down is left out: it fails nothing by design, a miss goes to the database", () => {
  const chaos = { cache: { down: true }, db: { down: true } };
  const kinds = { cache: "cache", db: "database" };
  expect(chaosHiddenHint(chaos, clean, kinds).map((h) => h.node)).toEqual(["db"]);
  // without kinds (older callers) nothing is skipped
  expect(chaosHiddenHint(chaos, clean).map((h) => h.node)).toEqual(["cache", "db"]);
});
