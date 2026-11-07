import { expect, test } from "vite-plus/test";
import type { EdgeDoc, Op } from "../../../data/api/types";
import { matchingPreset, PRESETS, presetOps } from "./presets";

const e = (more: Record<string, unknown> = {}): EdgeDoc => ({ from: "a", to: "b", ...more });

/** Applies the ops to a copy of the edge, as the draft would. */
function apply(edge: EdgeDoc, ops: Op[]): EdgeDoc {
  const next: Record<string, unknown> = { ...edge };
  for (const o of ops) {
    if (o.op !== "set" && o.op !== "insert" && o.op !== "remove") throw new Error("unexpected op");
    const key = String(o.path[2]);
    if (o.op === "remove") delete next[key];
    else next[key] = o.value;
  }
  return next as EdgeDoc;
}

test("three_basic_and_three_advanced_in_that_order", () => {
  expect(PRESETS.map((p) => `${p.level}:${p.id}`)).toEqual([
    "basic:typical",
    "basic:slow-link",
    "basic:lossy-link",
    "advanced:retry-budget",
    "advanced:breaker",
    "advanced:cold-connections",
  ]);
});

test("each_preset_writes_the_values_the_docs_give", () => {
  const w = (id: string, edge = e()) =>
    apply(
      edge,
      presetOps(
        PRESETS.find((p) => p.id === id)!,
        2,
        edge,
      ),
    );
  expect(w("slow-link").bandwidth).toBe(1024 * 1024); // 1 MB/s, KB = 1024 bytes
  expect(w("lossy-link").loss).toBe(0.01);
  expect(w("retry-budget").retry).toEqual({ max_attempts: 3, budget: {} });
  expect(w("breaker").breaker).toEqual({ window: 50, min_calls: 50, open_for: "10s" });
  const cold = w("cold-connections");
  expect([cold.tls, cold.keepalive]).toEqual([1.3, 1]);
});

test("typical_clears_every_setting_the_other_setups_write", () => {
  const busy = e({
    bandwidth: 5,
    loss: 0.01,
    tls: 1.3,
    keepalive: 1,
    compress: 0.3,
    stream: true,
    retry: {},
    abandon: "continue",
    breaker: {},
    calls: 3, // not a setup key: the wire's topology stays
  });
  const typical = PRESETS[0]!;
  const after = apply(busy, presetOps(typical, 0, busy));
  expect(after).toEqual(e({ calls: 3 }));
});

test("a_setup_changes_only_its_own_keys", () => {
  const edge = e({ loss: 0.02, calls: 4 });
  const after = apply(
    edge,
    presetOps(
      PRESETS.find((p) => p.id === "breaker")!,
      0,
      edge,
    ),
  );
  expect(after.loss).toBe(0.02);
  expect(after.calls).toBe(4);
});

test("applying_a_setup_twice_changes_nothing_the_second_time_and_the_chip_shows_pressed", () => {
  const retry = PRESETS.find((p) => p.id === "retry-budget")!;
  const once = apply(e(), presetOps(retry, 0, e()));
  expect(matchingPreset(once)).toBe("retry-budget");
  // a different value is no longer that setup
  expect(matchingPreset(apply(once, [{ op: "set", path: ["edges", 0, "retry"], value: {} }]))).toBe(
    null,
  );
  // the second application is a set of the same value: the edge ends the same
  expect(apply(once, presetOps(retry, 0, once))).toEqual(once);
});

test("a_wire_with_nothing_set_is_typical", () => {
  expect(matchingPreset(e())).toBe("typical");
  expect(matchingPreset(e({ calls: 3 }))).toBe("typical");
});

test("two_setups_at_once_match_none", () => {
  const both = apply(
    apply(e(), presetOps(PRESETS[1]!, 0, e())),
    presetOps(PRESETS[2]!, 0, apply(e(), presetOps(PRESETS[1]!, 0, e()))),
  );
  expect(matchingPreset(both)).toBe(null);
});
