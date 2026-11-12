import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { RAIL_MAX_W, RAIL_MIN_W, useRail } from "../../shell/layout/rail";
import { Rail } from "./Rail";

function show(tab: string, sentence = "", width = 1440) {
  vi.stubGlobal("innerWidth", width);
  history.replaceState(null, "", `/__kaanal/studio?tab=${tab}`);
  return render(
    <NuqsAdapter>
      <Rail
        sentence={sentence}
        panels={{ controls: <p>c</p>, log: <p>l</p>, results: <p>r</p>, live: <p>v</p> }}
      />
    </NuqsAdapter>,
  );
}

beforeEach(() => useRail.setState({ open: true }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("tabs_are_radix_with_aria_controls_and_arrow_keys", async () => {
  show("controls");
  const [controls, log] = screen.getAllByRole("tab");
  expect(controls?.getAttribute("aria-selected")).toBe("true");
  const panel = document.getElementById(controls?.getAttribute("aria-controls") ?? "");
  expect(panel?.getAttribute("role")).toBe("tabpanel");
  controls?.focus();
  fireEvent.keyDown(controls as Element, { key: "ArrowRight" });
  await waitFor(() => expect(document.activeElement).toBe(log));
});

test("the_rail_keeps_one_width_on_every_tab_and_stays_in_the_row", () => {
  for (const tab of ["controls", "log", "results", "live"]) {
    show(tab);
    expect(screen.getByRole("complementary").parentElement?.style.width).toBe("360px");
    expect(screen.getByRole("complementary").className).not.toMatch(/\babsolute\b/);
    cleanup();
  }
});

test("the_rail_folds_to_a_handle_with_the_button_or_the_bracket_key_and_remembers_it", () => {
  show("controls");
  fireEvent.click(screen.getByRole("button", { name: "Fold the details panel" }));
  expect(screen.queryByRole("complementary")).toBeNull();
  expect(useRail.getState().open).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Open the details panel" }));
  expect(screen.getByRole("complementary")).toBeTruthy();
  fireEvent.keyDown(window, { key: "]" });
  expect(screen.queryByRole("complementary")).toBeNull();
  fireEvent.keyDown(window, { key: "]" });
  expect(screen.getByRole("complementary")).toBeTruthy();
});

test("sentence_is_aria_live_polite_and_present_before_it_has_words", () => {
  const { rerender } = show("controls", "");
  const live = screen.getByTestId("rail-sentence");
  expect(live.getAttribute("aria-live")).toBe("polite");
  expect(live.textContent).toBe("");
  rerender(
    <NuqsAdapter>
      <Rail
        sentence="app: 78% busy"
        panels={{ controls: <p>c</p>, log: <p>l</p>, results: <p>r</p>, live: <p>v</p> }}
      />
    </NuqsAdapter>,
  );
  expect(screen.getByTestId("rail-sentence")).toBe(live);
  expect(live.textContent).toBe("app: 78% busy");
});

test("controls_tab_has_no_slots_per_instance_jargon_label", async () => {
  const { Controls } = await import("./Controls");
  render(
    <Controls
      name="app"
      config={{ type: "service", concurrency: 4 }}
      edges={[]}
      onDeleted={() => {}}
      onRenamed={() => {}}
      onChaos={() => {}}
    />,
  );
  expect(screen.queryByText(/per instance/i)).toBeNull();
  expect(screen.getByLabelText("Service time")).toBeTruthy();
});

test("tab_panels_contain_their_absolutely_positioned_children", () => {
  // the screen-reader-only heatmap table is absolute: without a positioned panel it sits against the
  // page and gives the whole window a scrollbar
  show("controls");
  const panel = screen.getByRole("tabpanel");
  expect(panel.className).toMatch(/\brelative\b/);
  expect(panel.className).toMatch(/overflow-y-auto/);
});

test("the_rail_width_is_kept_between_its_limits_and_moves_with_the_arrow_keys", () => {
  useRail.getState().setWidth(100);
  expect(useRail.getState().width).toBe(RAIL_MIN_W);
  useRail.getState().setWidth(9999);
  expect(useRail.getState().width).toBe(RAIL_MAX_W);
  useRail.getState().setWidth(360);
  useRail.getState().set(true);
  show("controls");
  const grip = screen.getByRole("separator", { name: "Resize the details panel" });
  fireEvent.keyDown(grip, { key: "ArrowLeft" });
  expect(useRail.getState().width).toBe(384);
  expect(localStorage.getItem("kaanal-rail-w")).toBe("384");
  fireEvent.keyDown(grip, { key: "ArrowRight" });
  expect(useRail.getState().width).toBe(360);
});
