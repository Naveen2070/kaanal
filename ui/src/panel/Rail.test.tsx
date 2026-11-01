import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { Rail } from "./Rail";

function show(tab: string, sentence = "", width = 1440) {
  vi.stubGlobal("innerWidth", width);
  history.replaceState(null, "", `/__kaanal/studio?tab=${tab}`);
  return render(
    <NuqsAdapter>
      <Rail sentence={sentence} panels={{ controls: <p>c</p>, log: <p>l</p>, results: <p>r</p> }} />
    </NuqsAdapter>,
  );
}

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

test("rail_is_300_on_controls_and_440_on_log_and_results", () => {
  const { unmount } = show("controls");
  expect(screen.getByRole("complementary").style.width).toBe("300px");
  unmount();
  show("log");
  expect(screen.getByRole("complementary").style.width).toBe("440px");
  cleanup();
  show("results");
  expect(screen.getByRole("complementary").style.width).toBe("440px");
});

test("rail_overlays_the_canvas_below_1280", () => {
  show("log", "", 1279);
  expect(screen.getByRole("complementary").className).toMatch(/\babsolute\b/);
  cleanup();
  show("log", "", 1280);
  expect(screen.getByRole("complementary").className).not.toMatch(/\babsolute\b/);
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
        panels={{ controls: <p>c</p>, log: <p>l</p>, results: <p>r</p> }}
      />
    </NuqsAdapter>,
  );
  expect(screen.getByTestId("rail-sentence")).toBe(live);
  expect(live.textContent).toBe("app: 78% busy");
});

test("rail_resize_is_instant_under_reduced_motion", () => {
  show("controls");
  const cls = screen.getByRole("complementary").className;
  expect(cls).toMatch(/transition-\[width\]/);
  expect(cls).toMatch(/motion-reduce:transition-none/);
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
  expect(screen.getByLabelText("Slots")).toBeTruthy();
});

test("tab_panels_contain_their_absolutely_positioned_children", () => {
  // the screen-reader-only heatmap table is absolute: without a positioned panel it sits against the
  // page and gives the whole window a scrollbar
  show("controls");
  const panel = screen.getByRole("tabpanel");
  expect(panel.className).toMatch(/\brelative\b/);
  expect(panel.className).toMatch(/overflow-y-auto/);
});
