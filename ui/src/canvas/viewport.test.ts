import { expect, test } from "vite-plus/test";
import { fitPadding, isVisible, padsFor, visibleCentre } from "./viewport";

const canvas = { left: 0, top: 0, right: 1440, bottom: 900 };

test("the_palette_takes_less_room_in_the_middle_layout", () => {
  expect(padsFor("full").left).toBeGreaterThan(padsFor("rail").left);
  expect(fitPadding("full")).toEqual({
    top: "168px",
    right: "392px",
    bottom: "96px",
    left: "328px",
  });
});

test("the_visible_centre_is_between_the_panels_not_the_middle_of_the_window", () => {
  const c = visibleCentre(canvas, padsFor("full"));
  expect(c.x).toBe((328 + 1440 - 392) / 2);
  expect(c.y).toBe((168 + 900 - 96) / 2);
});

test("a_box_behind_a_panel_is_not_visible", () => {
  const pads = padsFor("full");
  expect(isVisible({ left: 500, top: 300, right: 740, bottom: 480 }, canvas, pads)).toBe(true);
  expect(isVisible({ left: 100, top: 300, right: 340, bottom: 480 }, canvas, pads)).toBe(false); // under the palette
  expect(isVisible({ left: 500, top: 100, right: 740, bottom: 280 }, canvas, pads)).toBe(false); // under the top bar
  expect(isVisible({ left: 1200, top: 300, right: 1400, bottom: 480 }, canvas, pads)).toBe(false); // under the rail
});

test("a_new_card_goes_to_free_space_and_an_empty_spot_stays_put", async () => {
  const { freeSpot } = await import("./viewport");
  expect(freeSpot([100, 100], [])).toEqual([100, 100]);
  const p = freeSpot([100, 100], [[100, 100]]);
  expect(p).not.toEqual([100, 100]);
  expect(Math.abs(p[0] - 100) >= 260 || Math.abs(p[1] - 100) >= 210).toBe(true);
});

test("a_folded_rail_gives_its_room_back", () => {
  expect(padsFor("full", false).right).toBeLessThan(padsFor("full", true).right);
});
