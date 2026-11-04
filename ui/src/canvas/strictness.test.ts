import { expect, test } from "vite-plus/test";
import type { Doc } from "../data/api/types";
import { strictnessNote } from "./strictness";

const HOLD = "a slot held while waiting on a call (Server style)";
const edges = [{ from: "a", to: "b" }];
const doc = (over: Partial<Doc> = {}, a = {}, b = {}): Doc => ({
  nodes: { a: { type: "service", ...a }, b: { type: "database", ...b } },
  edges,
  ...over,
});

test("neither set says both", () => {
  expect(strictnessNote(doc())).toBe(
    `This run leaves out two limits: ${HOLD} and network bandwidth. A real system may saturate earlier.`,
  );
});

test("only hold set says bandwidth", () => {
  expect(strictnessNote(doc({}, { hold: "call" }))).toBe(
    "This run leaves out network bandwidth. A real system may saturate earlier.",
  );
});

test("only bandwidth set says hold", () => {
  expect(strictnessNote(doc({}, { bandwidth: 5000 }))).toBe(
    `This run leaves out ${HOLD}. A real system may saturate earlier.`,
  );
});

test("both set gives nothing", () => {
  expect(strictnessNote(doc({}, { hold: "call", bandwidth: 5000 }))).toBeNull();
});

test("a single component with no edge gives nothing", () => {
  expect(strictnessNote({ nodes: { a: { type: "service" } } })).toBeNull();
  expect(strictnessNote({ nodes: { a: { type: "service" } }, edges: [] })).toBeNull();
  expect(strictnessNote(undefined)).toBeNull();
});

test("a one-sided node bandwidth counts as set", () => {
  const note = strictnessNote(doc({}, { hold: "call" }, { bandwidth: { out: 1000 } }));
  expect(note).toBeNull();
});

test("an edge bandwidth counts as set", () => {
  const e = [{ from: "a", to: "b", bandwidth: 2000 }];
  expect(strictnessNote(doc({ edges: e }, { hold: "call" }))).toBeNull();
});

test("hold own is not hold call", () => {
  expect(strictnessNote(doc({}, { hold: "own" }))).toContain("two limits");
});

test("a_storage_node_in_the_draft_does_not_change_the_note", () => {
  const withStorage = doc({
    nodes: {
      a: { type: "service" },
      assets: {
        type: "storage",
        first_byte: { p50: "25ms", p99: "100ms" },
        throughput: "80MB/s",
      },
    },
    edges: [{ from: "a", to: "assets" }],
  });
  expect(strictnessNote(withStorage)).toBe(strictnessNote(doc()));
});
