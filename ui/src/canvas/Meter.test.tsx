// The meter (spec 4): an 8px track filled left to right at the true ratio; green, amber from 70%,
// red from 90%; the words above it in ink, never white on a fill.

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vite-plus/test";
import { Meter, meterTone } from "./Meter";

afterEach(cleanup);

const fill = () => document.querySelector("[data-fill]") as HTMLElement | null;

test("meter_fill_width_is_the_true_ratio_and_zero_has_no_stub", () => {
  render(<Meter ratio={0.78} label="78% busy" />);
  expect(fill()?.style.width).toBe("78%");
  cleanup();
  render(<Meter ratio={0} label="Idle" />);
  expect(fill()).toBeNull(); // no 1px sliver that reads as "a little busy"
  cleanup();
  render(<Meter ratio={1.7} label="x" />);
  expect(fill()?.style.width).toBe("100%"); // clamped
  cleanup();
  render(<Meter ratio={null} label="No slot limit · never saturates" />);
  expect(fill()).toBeNull();
  expect(screen.queryByRole("meter")).toBeNull(); // nothing to measure
});

test("meter_thresholds_pick_green_amber_red", () => {
  expect(meterTone(0)).toBe("ok");
  expect(meterTone(0.69)).toBe("ok");
  expect(meterTone(0.7)).toBe("warn");
  expect(meterTone(0.89)).toBe("warn");
  expect(meterTone(0.9)).toBe("bad");
  expect(meterTone(1)).toBe("bad");
  render(<Meter ratio={0.95} label="95% busy" />);
  expect(fill()?.className).toMatch(/bg-bad-fill/);
});

test("meter_text_is_ink_not_white", () => {
  render(<Meter ratio={0.95} label="95% busy · up to 3 waiting" />);
  const text = screen.getByText("95% busy · up to 3 waiting");
  expect(text.className).toMatch(/text-text/);
  expect(text.className).not.toMatch(/text-white/);
  expect(fill()?.textContent).toBe(""); // no words on the fill
});

test("the_meter_has_a_role_a_value_and_words_for_assistive_technology", () => {
  render(<Meter ratio={0.784} label="78% busy · up to 12 waiting" />);
  const m = screen.getByRole("meter", { name: "Busy" });
  expect(m.getAttribute("value")).toBe("78");
  expect(m.getAttribute("min")).toBe("0");
  expect(m.getAttribute("max")).toBe("100");
  expect(m.getAttribute("aria-valuetext")).toBe("78% busy · up to 12 waiting");
});

test("a_stale_meter_says_so_in_words", () => {
  render(<Meter ratio={0.5} label="50% busy" stale />);
  expect(screen.getByText("updating")).toBeTruthy();
  cleanup();
  render(<Meter ratio={0.5} label="50% busy" />);
  expect(screen.queryByText("updating")?.getAttribute("aria-hidden")).toBe("true");
});

test("the_fill_animation_stops_under_reduced_motion", () => {
  render(<Meter ratio={0.5} label="50% busy" />);
  expect(fill()?.className).toMatch(/motion-reduce:transition-none/);
});
