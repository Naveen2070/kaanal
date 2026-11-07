import { expect, test } from "vite-plus/test";
import { CHAPTERS } from "../guide/content";
import { HELP, HELP_LABEL, type HelpId } from "./copy";
import { WIRE_HELP } from "./copyWire";
import { WHERE } from "./where";

const ids = Object.keys(HELP) as HelpId[];
const entries = new Set(CHAPTERS.flatMap((c) => c.entries.map((e) => e.id)));

test("every_help_button_opens_a_glossary_entry_that_exists", () => {
  for (const id of ids) {
    expect(entries.has(HELP[id].entry ?? id), `${id} -> ${HELP[id].entry ?? id}`).toBe(true);
  }
});

test("every_help_entry_has_a_label_and_a_short_what", () => {
  for (const id of ids) {
    expect(HELP_LABEL[id], id).toBeTruthy();
    // spec 5.2: one plain sentence; a few of the wire ones carry a second clause, so 32 words
    expect(HELP[id].what.split(/\s+/).length, id).toBeLessThan(32);
  }
});

test("every_wire_term_gives_an_example_and_a_hint", () => {
  for (const [id, h] of Object.entries(WIRE_HELP)) {
    expect(h.example, id).toBeTruthy();
    expect(h.hint, id).toBeTruthy();
  }
});

test("the_wire_terms_are_all_in_the_merged_table", () => {
  for (const id of Object.keys(WIRE_HELP)) expect(HELP[id as HelpId], id).toBeTruthy();
});

test("every_where_line_belongs_to_a_wire_help_entry_and_reaches_its_glossary_entry", () => {
  for (const id of Object.keys(WHERE))
    expect(WIRE_HELP[id as keyof typeof WIRE_HELP], id).toBeTruthy();
  const entry = CHAPTERS.flatMap((c) => c.entries).find((e) => e.id === "retry");
  expect(entry?.where).toContain("When a call fails");
  expect(Object.keys(WHERE).length).toBeGreaterThanOrEqual(20);
});

test("network_latency_has_its_own_entry_that_says_it_is_charged_once", () => {
  expect(HELP["network-latency"].what).toContain("once per call");
  expect(HELP["network-latency"].example).toBeTruthy();
  expect(HELP["network-latency"].hint).toBeTruthy();
});
