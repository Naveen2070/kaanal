import { expect, test } from "vite-plus/test";
import { linePoints, niceTicks, smooth } from "./line";

test("niceTicks_round_steps_top_above_max", () => {
  expect(niceTicks(87, 4)).toEqual([0, 25, 50, 75, 100]);
  expect(niceTicks(0.7, 2)).toEqual([0, 0.5, 1]);
  expect(niceTicks(1280, 3)).toEqual([0, 500, 1000, 1500]);
});

test("a_zero_or_bad_max_gives_a_usable_scale", () => {
  for (const m of [0, -3, NaN, Infinity]) expect(niceTicks(m, 3)).toEqual([0, 1]);
});

test("zero_run_draws_a_flat_zero_line_not_nan", () => {
  expect(linePoints([], 1, 300, 120)).toBe("0,120 300,120");
  expect(linePoints([0, 0, 0], 1, 300, 120)).toBe("0,120 150,120 300,120");
  expect(linePoints([NaN], 1, 300, 120)).not.toMatch(/NaN/);
  expect(linePoints([5], 10, 300, 120)).toBe("0,60 300,60");
});

test("smoothing_turns_a_square_wave_into_a_trend_and_keeps_the_length", () => {
  const wave = [35, 50, 35, 50, 35, 50, 35, 50];
  const s = smooth(wave, 5);
  expect(s).toHaveLength(wave.length);
  expect(Math.max(...s) - Math.min(...s)).toBeLessThan(15);
  expect(smooth([NaN, 4, 6], 3)[0]).toBeNaN();
  expect(smooth([], 3)).toEqual([]);
});
