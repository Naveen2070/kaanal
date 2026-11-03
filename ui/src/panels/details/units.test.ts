import { expect, test } from "vite-plus/test";
import { fromUnit, pickUnit, RATE_UNITS, SIZE_UNITS, TIME_UNITS, toUnit } from "./units";

const [ms, s, min] = TIME_UNITS as [
  (typeof TIME_UNITS)[0],
  (typeof TIME_UNITS)[0],
  (typeof TIME_UNITS)[0],
];
const [kbps, mbps, gbps] = RATE_UNITS as [
  (typeof RATE_UNITS)[0],
  (typeof RATE_UNITS)[0],
  (typeof RATE_UNITS)[0],
];

test("a_value_converts_to_a_unit_and_back_without_noise", () => {
  expect(toUnit(1500, s)).toBe(1.5);
  expect(fromUnit(1.5, s)).toBe(1500);
  expect(toUnit(90_000, min)).toBe(1.5);
  expect(fromUnit(0.1 * 3, s)).toBe(300); // not 300.00000000000006
  expect(toUnit(2048, mbps)).toBe(2);
  expect(fromUnit(1, gbps)).toBe(1024 * 1024);
  expect(toUnit(125_000, kbps)).toBe(125_000);
});

test("a_value_shows_in_the_largest_unit_that_keeps_it_whole", () => {
  expect(pickUnit(TIME_UNITS, 25)).toBe(ms);
  expect(pickUnit(TIME_UNITS, 1000)).toBe(s);
  expect(pickUnit(TIME_UNITS, 1500)).toBe(ms); // 1.5 s is not whole
  expect(pickUnit(TIME_UNITS, 120_000)).toBe(min);
  expect(pickUnit(RATE_UNITS, 125_000)).toBe(kbps); // 122.07 MB/s is not whole
  expect(pickUnit(RATE_UNITS, 131_072)).toBe(mbps); // exactly 128 MB/s
  expect(pickUnit(SIZE_UNITS, 1024 * 1024)).toBe(SIZE_UNITS[2]);
});

test("nothing_to_show_gives_the_smallest_unit", () => {
  expect(pickUnit(TIME_UNITS, null)).toBe(ms);
  expect(pickUnit(TIME_UNITS, 0)).toBe(ms);
  expect(pickUnit(TIME_UNITS, Number.NaN)).toBe(ms);
});
