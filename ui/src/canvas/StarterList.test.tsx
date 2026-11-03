import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../data/api/types";
import { useDraft } from "../data/store/draft";
import { mockFetch } from "../test/fetch";
import { StarterList } from "./StarterList";
import { STARTERS } from "./starters";

const view = (text = ""): DraftView => ({
  text,
  dirty: false,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc: { nodes: {}, edges: [] },
  doc_error: null,
  project_id: "none",
  project_name: null,
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
});

beforeEach(() => useDraft.setState({ view: null, error: null }));
afterEach(cleanup);

test("a_starter_replaces_the_draft_with_its_whole_file", async () => {
  mockFetch([{ json: view() }]);
  await useDraft.getState().load();
  render(<StarterList />);
  const sent = mockFetch([{ json: view("x") }]);
  fireEvent.click(screen.getByRole("button", { name: /One app and a database/ }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.method).toBe("PUT");
  expect(JSON.stringify(sent[0]?.body)).toContain("type: monolith");
});

test("every_starter_says_what_to_try_and_the_written_ones_name_a_gateway_route", async () => {
  for (const s of STARTERS) {
    expect(s.tryThis.length, s.id).toBeGreaterThan(10);
    if (s.id !== "shop") {
      const text = await s.text();
      expect(text, s.id).toContain("type: gateway");
      expect(text, s.id).toContain("endpoints:");
      expect(text, s.id).toContain("traffic:");
    }
  }
});

test("a_refusal_is_shown_beside_the_list", async () => {
  mockFetch([{ json: view() }]);
  await useDraft.getState().load();
  render(<StarterList />);
  mockFetch([{ status: 400, json: { error: "nope" } }]);
  fireEvent.click(screen.getByRole("button", { name: /A fan-out/ }));
  await screen.findByRole("alert");
});
