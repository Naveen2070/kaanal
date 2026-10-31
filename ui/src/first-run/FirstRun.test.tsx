import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { useDraft } from "../store/draft";
import { mockFetch } from "../test/fetch";
import { TopBar } from "../topbar/TopBar";
import { Tour } from "./Tour";
import { neverAgain, projectKeyOf, shouldShowWelcome } from "./flags";
import { FirstRun } from "./FirstRun";
import { useFirstRun } from "./store";

function view(projectId: string, text = ""): DraftView {
  return {
    text,
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: { nodes: {} },
    doc_error: null,
    project_id: projectId,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
  };
}

async function open(projectId: string) {
  mockFetch([{ json: view(projectId) }]);
  await useDraft.getState().load();
  useFirstRun.setState({ phase: "idle", projectKey: "" });
  render(<FirstRun />);
}

const card = () => screen.queryByText("Welcome to Kaanal Studio");
const tour = () => screen.queryByRole("dialog", { name: /^Tour/ });

beforeEach(() => {
  localStorage.clear();
  useDraft.setState({ view: null, error: null, restarted: false });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  localStorage.clear();
});

test("shows_for_a_project_never_seen_and_not_again_after_close", async () => {
  await open("p1");
  expect(card()).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Start blank" }));
  expect(card()).toBeNull();
  cleanup();
  await open("p1");
  expect(card()).toBeNull();
  expect(tour()).toBeNull();
});

test("a_different_project_shows_it_again", async () => {
  await open("p1");
  fireEvent.click(screen.getByRole("button", { name: "Start blank" }));
  cleanup();
  await open("p2");
  expect(card()).toBeTruthy();
});

test("never_again_suppresses_card_and_tour_for_every_project", async () => {
  await open("p1");
  fireEvent.click(screen.getByRole("button", { name: "Don't show again" }));
  expect(card()).toBeNull();
  expect(tour()).toBeNull();
  expect(neverAgain()).toBe(true);
  cleanup();
  await open("p2");
  expect(card()).toBeNull();
  expect(tour()).toBeNull();
});

test("a_throwing_localStorage_defaults_to_show_and_does_not_crash", async () => {
  vi.stubGlobal("localStorage", {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  });
  expect(shouldShowWelcome("p1")).toBe(true);
  await open("p1");
  expect(card()).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Start blank" }));
  expect(tour()).toBeTruthy(); // closing still works, it just cannot be remembered
});

test("closing_the_card_starts_the_tour_and_skip_ends_it", async () => {
  await open("p1");
  fireEvent.click(screen.getByRole("button", { name: "Start blank" }));
  expect(tour()?.textContent).toMatch(/drag a component/);
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(tour()?.textContent).toMatch(/Select a component/);
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(tour()?.textContent).toMatch(/Run does it now/);
  fireEvent.click(screen.getByRole("button", { name: "Done" }));
  expect(tour()).toBeNull();
  // and Skip at any stop
  act(() => useFirstRun.getState().replayTour());
  fireEvent.click(screen.getByRole("button", { name: "Skip tour" }));
  expect(tour()).toBeNull();
});

test("the_question_mark_replays_the_tour_even_with_never_again", async () => {
  await open("p1");
  fireEvent.click(screen.getByRole("button", { name: "Don't show again" }));
  render(<TopBar onRun={() => {}} />);
  fireEvent.click(screen.getByRole("button", { name: "Show the tour" }));
  expect(tour()).toBeTruthy();
  fireEvent.keyDown(window, { key: "Escape" });
  expect(tour()).toBeNull();
});

test("try_the_demo_loads_the_demo_text_into_the_draft", async () => {
  await open("p1");
  const sent = mockFetch([
    { json: { text: "version: 1\nnodes: {}\n" } },
    { json: view("p1", "version: 1\nnodes: {}\n") },
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Try the demo" }));
  await vi.waitFor(() => expect(tour()).toBeTruthy());
  expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
    "GET /__kaanal/demo",
    "PUT /__kaanal/draft",
  ]);
  expect(sent[1]?.body).toEqual({ text: "version: 1\nnodes: {}\n" });
  expect(card()).toBeNull();
});

test("a_failed_demo_load_says_why_and_keeps_the_card", async () => {
  await open("p1");
  mockFetch([{ status: 500, json: { error: "no demo today" } }]);
  fireEvent.click(screen.getByRole("button", { name: "Try the demo" }));
  expect((await screen.findByRole("alert")).textContent).toBe("no demo today");
  expect(card()).toBeTruthy();
  expect((screen.getByRole("button", { name: "Try the demo" }) as HTMLButtonElement).disabled).toBe(
    false,
  );
});

test("the_card_traps_focus_and_escape_closes_it", async () => {
  await open("p1");
  const buttons = screen.getAllByRole("button");
  expect(document.activeElement).toBe(buttons[0]);
  // Tab from the last button wraps to the first, Shift+Tab from the first to the last
  const last = buttons[buttons.length - 1] as HTMLElement;
  last.focus();
  fireEvent.keyDown(last, { key: "Tab" });
  expect(document.activeElement).toBe(buttons[0]);
  fireEvent.keyDown(buttons[0] as HTMLElement, { key: "Tab", shiftKey: true });
  expect(document.activeElement).toBe(last);
  fireEvent.keyDown(last, { key: "Escape" });
  expect(card()).toBeNull();
  expect(tour()).toBeTruthy();
});

test("the_card_is_a_labelled_modal_dialog", async () => {
  await open("p1");
  const dialog = screen.getByRole("dialog", { name: "Welcome to Kaanal Studio" });
  expect(dialog.getAttribute("aria-modal")).toBe("true");
  expect(screen.getAllByRole("listitem")).toHaveLength(4);
});

test("a_draft_with_no_file_is_told_apart_by_the_page_origin", () => {
  expect(projectKeyOf("none")).toBe(location.origin);
  expect(projectKeyOf(undefined)).toBe(location.origin);
  expect(projectKeyOf("abc123")).toBe("abc123");
});

test("a_project_that_already_has_components_gets_no_card_and_the_demo_cannot_replace_it", async () => {
  const withNodes = { ...view("real"), doc: { nodes: { gw: { type: "gateway" } } } };
  mockFetch([{ json: withNodes }]);
  await useDraft.getState().load();
  useFirstRun.setState({ phase: "idle", projectKey: "" });
  render(<FirstRun />);
  expect(card()).toBeNull();
  expect(tour()).toBeNull();
  // the question mark still offers the tour
  act(() => useFirstRun.getState().replayTour());
  expect(tour()).toBeTruthy();
});

test("escape_while_the_demo_loads_does_nothing_and_the_tour_follows_the_load", async () => {
  await open("p1");
  mockFetch([
    { json: { text: "version: 1\nnodes: {}\n" } },
    { json: view("p1", "version: 1\nnodes: {}\n"), delayMs: 50 },
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Try the demo" }));
  const dialog = screen.getByRole("dialog", { name: "Welcome to Kaanal Studio" });
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(card()).toBeTruthy();
  const never = screen.getByRole("button", { name: "Don't show again" }) as HTMLButtonElement;
  expect(never.disabled).toBe(true);
  await vi.waitFor(() => expect(tour()).toBeTruthy());
});

test("an_escape_the_card_already_used_does_not_also_end_the_tour", () => {
  useFirstRun.setState({ phase: "tour", projectKey: "p" });
  render(<Tour />);
  const used = new KeyboardEvent("keydown", { key: "Escape", cancelable: true });
  used.preventDefault();
  window.dispatchEvent(used);
  expect(useFirstRun.getState().phase).toBe("tour");
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  expect(useFirstRun.getState().phase).toBe("off");
});

test("the_tour_focuses_next_and_gives_focus_back_to_run_when_it_ends", () => {
  const run = document.createElement("button");
  run.dataset.tour = "run";
  document.body.append(run);
  useFirstRun.setState({ phase: "tour", projectKey: "p" });
  const { unmount } = render(<Tour />);
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Next" }));
  unmount();
  expect(document.activeElement).toBe(run);
  run.remove();
});

function box(top: number, left: number, right: number, bottom: number): DOMRect {
  return {
    top,
    left,
    right,
    bottom,
    width: right - left,
    height: bottom - top,
    x: left,
    y: top,
    toJSON: () => ({}),
  };
}

test("a_tour_note_sits_beside_its_target_and_follows_a_resize", () => {
  const side = document.createElement("aside");
  side.dataset.tour = "palette";
  side.getBoundingClientRect = () => box(40, 0, 190, 600);
  document.body.append(side);
  useFirstRun.setState({ phase: "tour", projectKey: "p" });
  render(<Tour />);
  const note = screen.getByRole("dialog", { name: /^Tour/ }) as HTMLElement;
  expect(note.style.left).toBe("198px"); // right of the palette, not under it
  expect(note.style.top).toBe("48px");
  side.getBoundingClientRect = () => box(100, 0, 300, 600);
  act(() => void window.dispatchEvent(new Event("resize")));
  expect(note.style.left).toBe("308px");
  side.remove();
});
