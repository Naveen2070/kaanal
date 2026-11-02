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

test("every_sample_names_only_components_the_canvas_has", () => {
  const kinds = new Set(NODE_KINDS.map((k) => k.kind));
  const used: string[] = [];
  for (const e of all) {
    const s = e.sample;
    if (!s) continue;
    if (s.kind === "tile" || s.kind === "card") used.push(s.of);
    if (s.kind === "wire") used.push(s.from, s.to);
    if (s.kind === "chain") used.push(...s.steps.map((x) => x.of));
    if (s.kind === "calls") used.push(...s.rows.flatMap((r) => [r.from, ...r.to]));
  }
  expect(used.length).toBeGreaterThan(20);
  for (const k of used) expect(kinds.has(k), `kind ${k}`).toBe(true);
});

test("every_component_and_the_wiring_table_have_a_picture", () => {
  const components = CHAPTERS.find((c) => c.id === "components");
  for (const e of components?.entries ?? []) expect(e.sample?.kind, e.id).toBe("tile");
  expect(all.find((e) => e.id === "who-calls-whom")?.sample?.kind).toBe("calls");
  expect(all.filter((e) => e.sample).length).toBeGreaterThanOrEqual(25);
});
