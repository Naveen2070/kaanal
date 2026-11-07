import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { NodeProps } from "@xyflow/react";
import { afterEach, beforeAll, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView, NodeDoc } from "../data/api/types";
import { useDraft } from "../data/store/draft";
import { expectNoViolations } from "../test/axe";
import { mockFetch } from "../test/fetch";
import { nodeSummary } from "../test/summary";
import { ControlBar } from "./ControlBar";
import { EMPTY_FEED, FeedContext, type StatsFeed } from "./feed";
import {
  LOG_STEPS,
  msFromSlider,
  sliderFromMs,
  sliderFromSlots,
  slotsFromSlider,
} from "./logSlider";
import { capacityLine } from "./metrics";
import { NodeCard } from "./NodeCard";
import { setUpdating, useUpdating } from "./updating";

vi.mock("@xyflow/react", () => ({
  Handle: ({ type }: { type: string }) => <div data-handle={type} />,
  Position: { Left: "left", Right: "right" },
}));

const config: NodeDoc = { type: "service", concurrency: 12, proc: "98ms" };

beforeAll(() => {
  // jsdom has no layout, no ResizeObserver and no pointer capture: what Radix Slider needs to drag
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  Object.assign(Element.prototype, {
    setPointerCapture() {},
    releasePointerCapture() {},
    hasPointerCapture: () => true,
  });
  // a 1000px track from x = 0, so a pointer at x is slider position x
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    left: 0,
    top: 0,
    x: 0,
    y: 0,
    width: LOG_STEPS,
    height: 24,
    right: LOG_STEPS,
    bottom: 24,
    toJSON: () => ({}),
  });
});

function view(doc: DraftView["doc"]): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc,
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
  };
}

/** The draft store with `doc` loaded from the (mocked) server, then the answers to the edits. */
async function loaded(cfg: NodeDoc = config) {
  mockFetch([{ json: view({ nodes: { app: cfg } }) }]);
  await useDraft.getState().load();
  return mockFetch([{ json: view({ nodes: { app: cfg } }) }]);
}

const bar = (cfg: NodeDoc = config) => render(<ControlBar name="app" config={cfg} />);

function card(selected: boolean, cfg: NodeDoc = config, feed = EMPTY_FEED) {
  const props = (sel: boolean) =>
    ({ data: { name: "app", kind: cfg.type, config: cfg }, selected: sel }) as unknown as NodeProps;
  const r = render(
    <FeedContext.Provider value={feed}>
      <NodeCard {...props(selected)} />
    </FeedContext.Provider>,
  );
  return {
    select: (sel: boolean, f: StatsFeed = feed) =>
      r.rerender(
        <FeedContext.Provider value={f}>
          <NodeCard {...props(sel)} />
        </FeedContext.Provider>,
      ),
  };
}

const slider = (name: string) => screen.getByRole("slider", { name });
const typed = (name: string) =>
  screen.getByRole("spinbutton", { name: new RegExp(`^${name}, typed`) }) as HTMLInputElement;
const settings = () => screen.queryByRole("group", { name: "app settings" });

beforeEach(() => {
  useDraft.setState({ view: null, error: null, restarted: false });
  useUpdating.setState({ name: null });
});
afterEach(cleanup);

test("drag_moves_only_the_capacity_line_commit_on_release", async () => {
  const sent = await loaded();
  bar();
  expect(screen.getByText(capacityLine(12, 98, 1)!)).toBeTruthy();
  const thumb = slider("Slots");
  fireEvent.pointerDown(thumb, { pointerId: 1, clientX: 400 });
  fireEvent.pointerMove(thumb, { pointerId: 1, clientX: 500 });
  fireEvent.pointerMove(thumb, { pointerId: 1, clientX: 600 });
  // the line follows the thumb; nothing is sent while it moves
  expect(slotsFromSlider(600)).toBe(1000);
  expect(screen.getByText(capacityLine(1000, 98, 1)!)).toBeTruthy();
  expect(thumb.getAttribute("aria-valuetext")).toBe("1,000 slots at once");
  expect(sent).toHaveLength(0);
  fireEvent.pointerUp(thumb, { pointerId: 1, clientX: 600 });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "concurrency"], value: 1000 }],
  });
  // a key press is a move too: only its release commits
  fireEvent.keyDown(slider("Copies"), { key: "ArrowRight" });
  expect(sent).toHaveLength(1);
  fireEvent.keyUp(slider("Copies"), { key: "ArrowRight" });
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.body).toEqual({
    patch: [{ op: "insert", path: ["nodes", "app", "replicas"], value: 2 }],
  });
});

test("typed_zero_ms_is_accepted_and_warns_never_saturates", async () => {
  const sent = await loaded();
  bar();
  expect(screen.queryByText("0 ms never saturates")).toBeNull();
  const box = typed("Time");
  fireEvent.change(box, { target: { value: "0" } });
  fireEvent.keyDown(box, { key: "Enter" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "proc"], value: "0ms" }],
  });
  cleanup();
  bar({ ...config, proc: "0ms" });
  expect(screen.getByText("0 ms never saturates")).toBeTruthy();
  expect(screen.getByText(capacityLine(12, 0, 1)!)).toBeTruthy();
  expect(slider("Time").getAttribute("aria-valuetext")).toBe("0 milliseconds");
});

test("refused_value_puts_the_real_one_back", async () => {
  const sent = await loaded();
  bar();
  for (const [name, bad, real] of [
    ["Slots", "2.5", "12"],
    ["Slots", "0", "12"],
    ["Slots", "100001", "12"],
    ["Copies", "65", "1"],
    ["Copies", "", "1"],
    ["Time", "-5", "98"],
    ["Time", "60001", "98"],
  ] as const) {
    const box = typed(name);
    fireEvent.change(box, { target: { value: bad } });
    fireEvent.blur(box);
    expect(box.value, `${name} ${bad}`).toBe(real);
  }
  await useDraft.getState().settled();
  expect(sent).toHaveLength(0);
});

test("editing_copies_in_the_bar_patches_the_draft", async () => {
  const sent = await loaded();
  bar();
  const box = typed("Copies");
  fireEvent.change(box, { target: { value: "4" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["nodes", "app", "replicas"], value: 4 }],
  });
});

test("editing_the_service_time_sets_proc_and_a_bad_value_sends_nothing", async () => {
  const sent = await loaded();
  bar();
  const time = typed("Time");
  fireEvent.change(time, { target: { value: "" } });
  fireEvent.blur(time);
  fireEvent.change(time, { target: { value: "-5" } });
  fireEvent.blur(time);
  expect(sent).toHaveLength(0);
  fireEvent.change(time, { target: { value: "50" } });
  fireEvent.keyDown(time, { key: "Enter" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "proc"], value: "50ms" }],
  });
});

test("a_distribution_is_read_only_with_the_yaml_note", () => {
  bar({ type: "service", proc: { p50: "4ms", p99: "20ms" } });
  expect(screen.queryByRole("slider", { name: "Time" })).toBeNull();
  expect(screen.queryByRole("spinbutton", { name: /^Time/ })).toBeNull();
  expect(screen.getByText("time varies · edit in kaanal.yaml")).toBeTruthy();
  expect(slider("Slots")).toBeTruthy();
});

test("time_slider_is_log_from_0_1_ms_to_60_s", () => {
  expect(msFromSlider(0)).toBe(0.1);
  expect(msFromSlider(LOG_STEPS)).toBe(60_000);
  expect(sliderFromMs(0.1)).toBe(0);
  expect(sliderFromMs(60_000)).toBe(LOG_STEPS);
  expect(sliderFromMs(0)).toBe(0); // 0 ms is typed, the thumb sits at the low end
  expect(sliderFromMs(1e9)).toBe(LOG_STEPS);
  // log: the middle of the track is the geometric middle, about 77 ms
  expect(msFromSlider(500)).toBeCloseTo(Math.sqrt(0.1 * 60_000), 0);
  for (let ms = 0.1; ms <= 60_000; ms *= 1.37) {
    expect(Math.abs(msFromSlider(sliderFromMs(ms)) - ms) / ms, `${ms}`).toBeLessThan(0.01);
  }
  expect(slotsFromSlider(0)).toBe(1);
  expect(slotsFromSlider(LOG_STEPS)).toBe(100_000);
  for (const n of [1, 2, 7, 12, 32, 96, 256, 4096, 99_999]) {
    expect(Math.abs(slotsFromSlider(sliderFromSlots(n)) - n) / n, `${n}`).toBeLessThan(0.012);
  }
});

test("slider_keys_arrows_pagekeys_home_end", async () => {
  const sent = await loaded();
  bar();
  const t = slider("Time");
  const start = sliderFromMs(98);
  const at = () => Number(t.getAttribute("aria-valuenow"));
  expect(at()).toBe(start);
  for (const [key, want] of [
    ["ArrowRight", start + 1],
    ["ArrowUp", start + 2],
    ["ArrowLeft", start + 1],
    ["ArrowDown", start],
    ["PageUp", start + 10],
    ["PageDown", start],
    ["Home", 0],
    ["End", LOG_STEPS],
  ] as const) {
    fireEvent.keyDown(t, { key });
    expect(at(), key).toBe(want);
  }
  expect(t.getAttribute("aria-valuetext")).toBe("60,000 milliseconds");
  fireEvent.keyUp(t, { key: "Tab" }); // a Tab is not a slider key
  expect(sent).toHaveLength(0);
  fireEvent.keyUp(t, { key: "End" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "proc"], value: "60000ms" }],
  });
});

test("a_key_step_on_slots_moves_to_the_next_whole_number", () => {
  bar();
  const s = slider("Slots");
  // near 12 several log positions round to 12: a press must still change the value
  fireEvent.keyDown(s, { key: "ArrowRight" });
  expect(s.getAttribute("aria-valuetext")).toBe("13 slots at once");
  fireEvent.keyDown(s, { key: "ArrowLeft" });
  fireEvent.keyDown(s, { key: "ArrowLeft" });
  expect(s.getAttribute("aria-valuetext")).toBe("11 slots at once");
  fireEvent.keyDown(s, { key: "Home" });
  fireEvent.keyDown(s, { key: "ArrowLeft" }); // already at the end: stays
  expect(s.getAttribute("aria-valuetext")).toBe("1 slot at once");
});

test("tabbing_onto_a_slider_or_moving_it_back_patches_nothing", async () => {
  const sent = await loaded();
  bar();
  const s = slider("Slots");
  fireEvent.keyUp(s, { key: "ArrowRight" }); // a release with no move
  fireEvent.keyDown(s, { key: "ArrowRight" });
  fireEvent.keyDown(s, { key: "ArrowLeft" });
  fireEvent.keyUp(s, { key: "ArrowLeft" }); // back on the real value's position
  fireEvent.pointerUp(s);
  await useDraft.getState().settled();
  expect(sent).toHaveLength(0);
});

test("rows_have_labels_values_in_words_and_help_triggers", () => {
  bar({ ...config, concurrency: 32, proc: "25ms", replicas: 3 });
  expect(slider("Time").getAttribute("aria-valuetext")).toBe("25 milliseconds");
  expect(slider("Slots").getAttribute("aria-valuetext")).toBe("32 slots at once");
  expect(slider("Copies").getAttribute("aria-valuetext")).toBe("3 copies");
  for (const row of ["Time", "Slots", "Copies"]) {
    const help = screen.getByRole("button", { name: `About ${row}` });
    expect(help.className).toContain("size-6"); // 24px (WCAG 2.5.8)
  }
  for (const thumb of screen.getAllByRole("slider")) expect(thumb.className).toContain("size-6");
});

test("database_card_has_no_time_row", () => {
  bar({ type: "database", pool: { size: 10 } });
  expect(screen.queryByRole("slider", { name: "Time" })).toBeNull();
  expect(screen.queryByRole("button", { name: "About Time" })).toBeNull();
  expect(slider("Slots").getAttribute("aria-valuetext")).toBe("10 slots at once");
  expect(slider("Copies")).toBeTruthy();
  expect(screen.queryByText(/Can finish/)).toBeNull(); // its cost comes from its flavor
});

test("capacity_line_text_matches_the_nodeFigures_selector", () => {
  const cfg = { type: "service", concurrency: 32, proc: "25ms", replicas: 1 };
  bar(cfg);
  // one helper words the line for the bar and for every other place that says it
  const line = capacityLine(32, 25, 1);
  expect(line).toBe("Can finish up to 1,280/s · 32 at once ÷ 25 ms");
  expect(screen.getByText(line!)).toBeTruthy();
});

// ---- review fixes ----

test("enter_in_a_typed_box_keeps_keyboard_focus_there", async () => {
  const sent = await loaded();
  const r = bar();
  const slots = typed("Slots");
  act(() => slots.focus());
  fireEvent.change(slots, { target: { value: "20" } });
  fireEvent.keyDown(slots, { key: "Enter" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toMatchObject({
    patch: [{ op: "set", path: ["nodes", "app", "concurrency"], value: 20 }],
  });
  // the draft answers with the new value and the card hands the bar the new config
  r.rerender(<ControlBar name="app" config={{ ...config, concurrency: 20 }} />);
  expect(typed("Slots")).toBe(slots); // the same element, not a rebuilt one
  expect(document.activeElement).toBe(slots);
  expect(slots.value).toBe("20");
});

test("a_drag_that_loses_pointer_capture_still_commits_once", async () => {
  const sent = await loaded();
  bar();
  fireEvent.keyDown(slider("Slots"), { key: "ArrowRight" });
  const root = slider("Slots").closest("[data-orientation]") as HTMLElement;
  fireEvent.lostPointerCapture(root);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  fireEvent.pointerUp(slider("Slots")); // and a late release adds nothing
  fireEvent.lostPointerCapture(root);
  await act(async () => {});
  expect(sent).toHaveLength(1);
});

test("a_time_below_a_microsecond_is_refused_and_the_slider_shows_only_what_it_can_save", async () => {
  const sent = await loaded();
  bar();
  fireEvent.change(typed("Time"), { target: { value: "0.0004" } });
  fireEvent.blur(typed("Time"));
  expect(sent).toHaveLength(0);
  expect(typed("Time").value).toBe("98");
  // every slider position gives a time the draft can hold exactly (three decimals at most)
  for (let p = 0; p <= LOG_STEPS; p++) {
    const ms = msFromSlider(p);
    expect(Number(ms.toFixed(3)), `position ${p}`).toBe(ms);
  }
  // 0 is still allowed
  fireEvent.change(typed("Time"), { target: { value: "0" } });
  fireEvent.blur(typed("Time"));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
});

/** A card and the dock's bar for the same node, the way the page puts them together. */
function cardAndBar(feed: StatsFeed, cfg: NodeDoc = config) {
  const el = (f: StatsFeed) => (
    <FeedContext.Provider value={f}>
      <NodeCard
        {...({
          data: { name: "app", kind: cfg.type, config: cfg },
          selected: true,
        } as unknown as NodeProps)}
      />
      <ControlBar dock name="app" config={cfg} onPending={(on) => setUpdating("app", on)} />
    </FeedContext.Provider>
  );
  const r = render(el(feed));
  return { feed: (f: StatsFeed) => r.rerender(el(f)) };
}

test("the_card_holds_no_settings_and_no_bar_opens_under_it", () => {
  card(true);
  expect(settings()).toBeNull();
  expect(screen.queryByRole("slider")).toBeNull();
});

test("the_card_keeps_its_handles_in_one_part_whatever_is_selected", () => {
  card(true);
  const parts = [...document.querySelectorAll("[data-handle]")].map(
    (h) => h.parentElement?.getAttribute("data-part") ?? "elsewhere",
  );
  expect(parts).toEqual(["card", "card"]);
});

test("the_dock_lays_the_rows_in_columns_by_its_own_width_and_the_drawer_stacks_them", () => {
  const r = bar();
  expect(settings()?.querySelector(".grid")).toBeNull();
  r.unmount();
  render(<ControlBar dock name="app" config={config} />);
  const grid = settings()?.querySelector(".grid");
  expect(grid).not.toBeNull();
  // one column when narrow, two from 30 rem, three from 46 rem; a lone last row takes the line (two columns)
  expect(grid?.className).toContain("grid-cols-1");
  expect(grid?.className).toContain("@min-[30rem]:grid-cols-2");
  expect(grid?.className).toContain("@min-[46rem]:grid-cols-3");
  expect(grid?.className).toContain("@min-[30rem]:[&>:last-child:nth-child(odd)]:col-span-2");
  expect(grid?.className).toContain("@min-[46rem]:[&>:last-child:nth-child(odd)]:col-span-1");
  expect(settings()?.classList.contains("nodrag") && settings()?.classList.contains("nopan")).toBe(
    true,
  );
});

test("dragging_marks_the_meter_updating_until_the_run_starts", async () => {
  await loaded();
  const ran = nodeSummary({ name: "app" });
  const feed = (dimmed: boolean, n = ran): StatsFeed => ({
    node: (name) => (name === "app" ? n : undefined),
    nodes: [n],
    dimmed,
    peakVisits: 0,
  });
  const c = cardAndBar(feed(false));
  expect(screen.queryByText("updating")?.getAttribute("aria-hidden")).toBe("true");
  fireEvent.keyDown(slider("Slots"), { key: "ArrowRight" });
  expect(screen.getByText("updating")).toBeTruthy();
  fireEvent.keyUp(slider("Slots"), { key: "ArrowRight" });
  await useDraft.getState().settled();
  expect(screen.getByText("updating")).toBeTruthy(); // the debounced run has not started yet
  c.feed(feed(true)); // it starts: the feed marks the figures
  expect(screen.getByText("updating")).toBeTruthy();
  c.feed(feed(false, nodeSummary({ name: "app" }))); // it landed
  expect(screen.queryByText("updating")?.getAttribute("aria-hidden")).toBe("true");
});

test("without_figures_there_is_no_updating_marker_to_get_stuck", async () => {
  await loaded();
  cardAndBar(EMPTY_FEED); // no run at all (a draft with no traffic never runs)
  fireEvent.keyDown(slider("Slots"), { key: "ArrowRight" });
  expect(screen.queryByText("updating")?.getAttribute("aria-hidden")).toBe("true");
});

test("escape_in_a_typed_box_cancels_it_writes_nothing_and_asks_to_close", async () => {
  const sent = await loaded();
  const onClose = vi.fn();
  render(<ControlBar dock name="app" config={config} onClose={onClose} />);
  const time = typed("Time");
  act(() => time.focus());
  fireEvent.change(time, { target: { value: "500" } });
  fireEvent.keyDown(time, { key: "Escape" });
  act(() => time.blur()); // the dock closing moves focus away, and a blur would commit
  await act(async () => {});
  expect(sent).toHaveLength(0);
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("escape_does_not_clear_updating_while_an_edit_is_on_its_way", async () => {
  await loaded();
  const ran = nodeSummary({ name: "app" });
  cardAndBar({
    node: (n) => (n === "app" ? ran : undefined),
    nodes: [ran],
    dimmed: false,
    peakVisits: 0,
  });
  fireEvent.keyDown(slider("Slots"), { key: "ArrowRight" });
  fireEvent.keyUp(slider("Slots"), { key: "ArrowRight" }); // committed: the run is pending
  expect(screen.getByText("updating")).toBeTruthy();
  fireEvent.keyDown(slider("Slots"), { key: "Escape" });
  expect(screen.getByText("updating")).toBeTruthy();
});

test("an_open_bar_has_no_axe_violations", async () => {
  render(<ControlBar dock name="app" config={config} />);
  await expectNoViolations(document.body);
  cleanup();
  render(<ControlBar dock name="app" config={{ ...config, proc: "0ms" }} />);
  await expectNoViolations(document.body);
  cleanup();
  render(
    <ControlBar dock name="app" config={{ type: "service", proc: { p50: "4ms", p99: "20ms" } }} />,
  );
  await expectNoViolations(document.body);
});

// ---- Chrome-pass findings ----

test("typed_text_gives_way_when_the_config_changes_from_outside", async () => {
  await loaded();
  const r = bar();
  fireEvent.change(typed("Slots"), { target: { value: "10" } }); // typed, not committed, not focused
  expect(typed("Slots").value).toBe("10");
  r.rerender(<ControlBar name="app" config={{ ...config, concurrency: 8 }} />); // reload, undo
  expect(typed("Slots").value).toBe("8");
});

test("typed_text_is_kept_while_the_box_is_being_edited", async () => {
  await loaded();
  const r = bar();
  act(() => typed("Slots").focus());
  fireEvent.change(typed("Slots"), { target: { value: "10" } });
  r.rerender(<ControlBar name="app" config={{ ...config, concurrency: 8 }} />);
  expect(typed("Slots").value).toBe("10");
});

test("typed_boxes_have_a_real_max_and_sliders_say_their_value", async () => {
  await loaded({ ...config, replicas: 3 });
  bar({ ...config, replicas: 3 });
  for (const n of ["Slots", "Copies", "Time"]) {
    expect(Number(typed(n).max), n).toBeGreaterThan(0);
  }
  expect(slider("Slots").getAttribute("aria-valuetext")).toBe("12 slots at once");
  expect(slider("Copies").getAttribute("aria-valuetext")).toBe("3 copies");
});
