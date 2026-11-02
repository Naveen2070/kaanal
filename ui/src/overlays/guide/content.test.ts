import { expect, test } from "vite-plus/test";
import { NODE_KINDS } from "../../canvas/nodeKinds";
import { HELP } from "../help/copy";
import { CHAPTERS, searchEntries } from "./content";

const all = CHAPTERS.flatMap((c) => c.entries);

test("chapters_and_entries_have_unique_ids_and_real_text", () => {
  expect(new Set(CHAPTERS.map((c) => c.id)).size).toBe(CHAPTERS.length);
  for (const c of CHAPTERS) {
    expect(c.title.length).toBeGreaterThan(0);
    expect(c.intro.length).toBeGreaterThan(20);
    expect(new Set(c.entries.map((e) => e.id)).size).toBe(c.entries.length);
    for (const e of c.entries) {
      expect(e.term.trim()).not.toBe("");
      expect(e.text.trim().length).toBeGreaterThan(20);
    }
  }
});

test("every_help_word_and_every_component_kind_has_an_entry", () => {
  const ids = new Set(all.map((e) => e.id));
  for (const id of Object.keys(HELP)) expect(ids.has(id), `help word ${id}`).toBe(true);
  for (const k of NODE_KINDS) expect(ids.has(k.kind), `kind ${k.kind}`).toBe(true);
});

test("every_component_says_what_it_is_in_a_real_system", () => {
  const components = CHAPTERS.find((c) => c.id === "components");
  expect(components?.entries.length).toBe(NODE_KINDS.length);
  for (const e of components?.entries ?? []) expect(e.real?.length ?? 0).toBeGreaterThan(10);
});

test("search_needs_every_word_and_looks_in_the_real_world_note", () => {
  expect(searchEntries("").length).toBe(0);
  expect(searchEntries("breaks first").map((r) => r.entry.id)).toContain("breaks-first");
  // 'redis' appears only in the cache's real-world note
  expect(searchEntries("redis").map((r) => r.entry.id)).toEqual(["cache"]);
  expect(searchEntries("zzzz nothing")).toEqual([]);
});
