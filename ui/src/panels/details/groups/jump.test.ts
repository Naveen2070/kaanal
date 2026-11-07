import { expect, test } from "vite-plus/test";
import { matchTargets, WIRE_TARGETS } from "./jump";

test("typing_retry_finds_the_retry_boxes_first", () => {
  const found = matchTargets(WIRE_TARGETS, "retry").map((t) => t.label);
  expect(found[0]).toBe("Retry a failed call");
  expect(found).toContain("Tries (the first counts)");
  expect(found).not.toContain("Packet loss (%)");
});

test("every_word_typed_must_match_in_any_order_and_case", () => {
  expect(matchTargets(WIRE_TARGETS, "WAIT longest").map((t) => t.label)).toEqual(["Longest wait"]);
});

test("a_synonym_finds_its_setting", () => {
  expect(matchTargets(WIRE_TARGETS, "tls").map((t) => t.label)).toContain("Secure connection");
  expect(matchTargets(WIRE_TARGETS, "gzip").map((t) => t.label)).toContain(
    "Compress the response (%)",
  );
});

test("an_empty_or_blank_search_finds_nothing_and_a_miss_finds_nothing", () => {
  expect(matchTargets(WIRE_TARGETS, "")).toEqual([]);
  expect(matchTargets(WIRE_TARGETS, "   ")).toEqual([]);
  expect(matchTargets(WIRE_TARGETS, "zzzz")).toEqual([]);
});

test("at_most_six_are_offered", () => {
  expect(matchTargets(WIRE_TARGETS, "a").length).toBeLessThanOrEqual(6);
});

test("a_target_in_a_group_names_a_known_group", () => {
  const known = new Set([
    "wire.connection",
    "wire.compress",
    "wire.retry",
    "wire.breaker",
    "wire.usage",
  ]);
  for (const t of WIRE_TARGETS) if (t.group) expect(known.has(t.group), t.label).toBe(true);
});
