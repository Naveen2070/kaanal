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
  fireEvent.click(screen.getByRole("button", { name: /^One app and a database/ }));
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
  fireEvent.click(screen.getByRole("button", { name: /^A fan-out/ }));
  await screen.findByRole("alert");
});

test("every_starter_card_has_a_picture_and_the_picture_is_not_read_out", () => {
  const { container } = render(<StarterList />);
  const pictures = container.querySelectorAll("li svg[aria-hidden]");
  expect(pictures.length).toBe(STARTERS.length * 1 + STARTERS.length); // a sketch and an info icon each
  for (const s of STARTERS) {
    const card = screen.getByRole("button", { name: new RegExp(`^${s.title}`) });
    expect(card.querySelector("svg[aria-hidden]"), s.id).toBeTruthy();
  }
});

test("the_try_line_is_cut_short_and_its_round_button_opens_the_whole_line", () => {
  render(<StarterList />);
  const card = screen.getByRole("button", { name: /^A retry storm/ });
  const line = [...card.querySelectorAll("span")].find((e) => e.textContent?.startsWith("Try:"));
  expect(line?.className).toContain("truncate");
  const more = screen.getByRole("button", { name: "About A retry storm" });
  expect(more.getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(more);
  expect(more.getAttribute("aria-expanded")).toBe("true");
  expect(line?.className).not.toContain("truncate");
  expect(line?.textContent).toContain("circuit breaker");
  fireEvent.click(more);
  expect(line?.className).toContain("truncate");
});

test("the_header_holds_the_action_the_caller_gives", () => {
  render(<StarterList action={<button type="button">or start with a gateway</button>} />);
  const head = screen.getByRole("heading", { name: "Start from a system" }).parentElement;
  expect(head?.textContent).toContain("or start with a gateway");
});

test("the_list_scrolls_inside_its_own_box_and_takes_two_columns_by_its_own_width", () => {
  const { container } = render(<StarterList />);
  const list = container.querySelector("ul");
  expect(list?.className).toContain("overflow-y-auto");
  expect(list?.className).toContain("@min-[35rem]:grid-cols-2");
  expect(container.firstElementChild?.className).toContain("@container");
  expect(container.firstElementChild?.className).toContain("max-h-full");
});

test("the_written_starters_bound_every_queue_and_give_the_gateway_a_30s_timeout", async () => {
  for (const s of STARTERS.filter((x) => x.id !== "shop")) {
    const text = await s.text();
    expect(text, s.id).not.toMatch(/concurrency: \d/); // the long form only
    const slotted = text.match(/concurrency: \{ limit: \d+, queue: 100 \}/g) ?? [];
    expect(slotted.length, s.id).toBeGreaterThan(1);
    expect(text.match(/concurrency: \{/g)?.length, s.id).toBe(slotted.length);
    expect(text, s.id).toMatch(/gateway: \{ type: gateway,[^\n]*timeout: 30s \}/);
  }
});

test("every_card_is_one_height_so_a_short_description_leaves_no_small_card_and_the_info_button_stays_inside", () => {
  const { container } = render(<StarterList />);
  const list = container.querySelector("ul");
  // rows share the height of the tallest, and each card fills its row
  expect(list?.className).toContain("auto-rows-fr");
  for (const card of container.querySelectorAll("li > button:first-child")) {
    expect(card.className, card.textContent ?? "").toContain("h-full");
  }
});
