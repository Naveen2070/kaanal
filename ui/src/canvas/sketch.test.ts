import { expect, test } from "vite-plus/test";
import { catColour } from "./nodeKinds";
import { layoutSketch, SKETCH_H, SKETCH_W } from "./sketch";
import { STARTERS } from "./starters";

test("a_chain_runs_left_to_right_on_one_row", () => {
  const s = layoutSketch({
    nodes: [
      { id: "a", kind: "gateway" },
      { id: "b", kind: "service" },
      { id: "c", kind: "database" },
    ],
    wires: [
      ["a", "b"],
      ["b", "c"],
    ],
  });
  const [a, b, c] = s.nodes;
  expect(a!.x).toBeLessThan(b!.x);
  expect(b!.x).toBeLessThan(c!.x);
  expect([a!.y, b!.y, c!.y]).toEqual([SKETCH_H / 2, SKETCH_H / 2, SKETCH_H / 2]);
  expect(s.wires).toHaveLength(2);
});

test("a_column_is_the_longest_way_from_a_source_so_a_join_sits_after_both_its_callers", () => {
  // a -> b -> d and a -> d: d is two steps from a, not one
  const s = layoutSketch({
    nodes: ["a", "b", "d"].map((id) => ({ id, kind: "service" })),
    wires: [
      ["a", "b"],
      ["b", "d"],
      ["a", "d"],
    ],
  });
  const x = (id: string) => s.nodes.find((n) => n.id === id)!.x;
  expect(x("d")).toBeGreaterThan(x("b"));
});

test("copies_are_drawn_as_stacked_boxes_each_with_its_own_wires", () => {
  const s = layoutSketch({
    nodes: [
      { id: "lb", kind: "load-balancer" },
      { id: "api", kind: "service", copies: 3 },
    ],
    wires: [["lb", "api"]],
  });
  expect(s.nodes.filter((n) => n.id.startsWith("api"))).toHaveLength(3);
  expect(s.wires).toHaveLength(3);
  const ys = s.nodes.filter((n) => n.id.startsWith("api")).map((n) => n.y);
  expect(new Set(ys).size).toBe(3);
});

test("a_failing_wire_is_marked", () => {
  const s = layoutSketch({
    nodes: [
      { id: "a", kind: "gateway" },
      { id: "b", kind: "service" },
    ],
    wires: [["a", "b"]],
    failing: [["a", "b"]],
  });
  expect(s.wires[0]?.failing).toBe(true);
});

test("every_starter_has_a_sketch_that_fits_its_box_with_no_two_boxes_on_top_of_each_other", () => {
  for (const st of STARTERS) {
    const s = layoutSketch(st.graph);
    expect(s.nodes.length, st.id).toBeGreaterThanOrEqual(2);
    expect(s.wires.length, st.id).toBeGreaterThanOrEqual(1);
    for (const n of s.nodes) {
      expect(n.x - 6, `${st.id} ${n.id}`).toBeGreaterThanOrEqual(0);
      expect(n.x + 6, `${st.id} ${n.id}`).toBeLessThanOrEqual(SKETCH_W);
      expect(n.y - 4.5, `${st.id} ${n.id}`).toBeGreaterThanOrEqual(0);
      expect(n.y + 4.5, `${st.id} ${n.id}`).toBeLessThanOrEqual(SKETCH_H);
    }
    const spots = s.nodes.map((n) => `${n.x.toFixed(1)},${n.y.toFixed(1)}`);
    expect(new Set(spots).size, st.id).toBe(spots.length);
  }
});

test("every_kind_in_a_starter_has_a_colour_of_its_own", () => {
  for (const st of STARTERS) {
    for (const n of st.graph.nodes) {
      expect(catColour(n.kind), `${st.id} ${n.kind}`).not.toBe("var(--muted)");
    }
  }
});

test("only_the_retry_storm_draws_a_failing_wire", () => {
  for (const st of STARTERS) {
    expect((st.graph.failing ?? []).length > 0, st.id).toBe(st.id === "retry-storm");
  }
});

test("a_written_starters_sketch_matches_its_file", async () => {
  for (const st of STARTERS) {
    if (st.id === "shop") continue; // the server's own demo: its text is fetched, not written here
    const text = await st.text();
    for (const n of st.graph.nodes) {
      expect(text, `${st.id} node ${n.id}`).toContain(`  ${n.id}: { type: ${n.kind}`);
      if (n.copies) expect(text, `${st.id} copies of ${n.id}`).toContain(`replicas: ${n.copies}`);
    }
    for (const [from, to] of st.graph.wires) {
      expect(text, `${st.id} wire ${from} to ${to}`).toContain(`{ from: ${from}, to: ${to}`);
    }
    for (const [from, to] of st.graph.failing ?? []) {
      const line = text.split("\n").find((l) => l.includes(`{ from: ${from}, to: ${to}`));
      expect(line, `${st.id} failing wire ${from} to ${to}`).toContain("retry:");
    }
  }
});
