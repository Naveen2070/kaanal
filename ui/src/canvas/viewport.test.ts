import { expect, test } from "vite-plus/test";
import { fitPadding, isVisible, padsFor, visibleCentre } from "./viewport";

const canvas = { left: 0, top: 0, right: 1440, bottom: 900 };

test("the_palette_takes_less_room_in_the_middle_layout", () => {
  expect(padsFor("full").left).toBeGreaterThan(padsFor("rail").left);
  expect(fitPadding("full")).toEqual({
    top: "176px",
    right: "340px",
    bottom: "96px",
    left: "272px",
  });
});

test("the_visible_centre_is_between_the_panels_not_the_middle_of_the_window", () => {
  const c = visibleCentre(canvas, padsFor("full"));
  expect(c.x).toBe((272 + 1440 - 340) / 2);
  expect(c.y).toBe((176 + 900 - 96) / 2);
});

test("a_box_behind_a_panel_is_not_visible", () => {
  const pads = padsFor("full");
  expect(isVisible({ left: 500, top: 300, right: 740, bottom: 480 }, canvas, pads)).toBe(true);
  expect(isVisible({ left: 100, top: 300, right: 340, bottom: 480 }, canvas, pads)).toBe(false); // under the palette
  expect(isVisible({ left: 500, top: 100, right: 740, bottom: 280 }, canvas, pads)).toBe(false); // under the top bar
  expect(isVisible({ left: 1200, top: 300, right: 1400, bottom: 480 }, canvas, pads)).toBe(false); // under the rail
});
