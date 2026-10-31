import { expect, test } from "vite-plus/test";
import { rpsFromSlider, sliderFromRps } from "./logScale";

test("slider_ends_map_to_10_and_15000", () => {
  expect(rpsFromSlider(0)).toBe(10);
  expect(rpsFromSlider(1000)).toBe(15000);
  expect(sliderFromRps(10)).toBe(0);
  expect(sliderFromRps(15000)).toBe(1000);
});

test("log_scale_round_trips_within_one_percent", () => {
  for (const rps of [10, 11, 13, 17, 25, 99, 100, 250, 1280, 4321, 9999, 15000]) {
    const back = rpsFromSlider(sliderFromRps(rps));
    expect(Math.abs(back - rps) / rps, `${rps} -> ${back}`).toBeLessThanOrEqual(0.01);
  }
});

test("the_scale_is_logarithmic_the_middle_is_the_geometric_mean", () => {
  // sqrt(10 * 15000) is about 387; a linear slider would sit at 7505
  expect(rpsFromSlider(500)).toBeGreaterThan(380);
  expect(rpsFromSlider(500)).toBeLessThan(395);
});

test("out_of_range_and_junk_are_held_to_the_ends", () => {
  expect(rpsFromSlider(-50)).toBe(10);
  expect(rpsFromSlider(5000)).toBe(15000);
  expect(rpsFromSlider(Number.NaN)).toBe(10);
  expect(sliderFromRps(1)).toBe(0); // below the slider's range: pinned to the left end
  expect(sliderFromRps(1_000_000)).toBe(1000); // typed past the range: pinned to the right end
  expect(sliderFromRps(Number.NaN)).toBe(0);
});
