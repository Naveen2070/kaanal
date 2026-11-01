import { expect, test } from "vite-plus/test";
import { bendX } from "./routing";

test("with_nothing_in_the_way_the_wire_bends_halfway", () => {
  expect(bendX(0, 400, 0, 200, [])).toBe(200);
});

test("a_card_in_the_middle_column_moves_the_bend_to_a_clear_gap", () => {
  // a card at x 150..250 spans the wire's rows; the clearest gaps are either side of it
  const x = bendX(0, 400, 0, 200, [{ x: 150, y: 50, w: 100, h: 100 }]);
  expect(x).not.toBe(200);
  expect(x < 136 || x > 264).toBe(true);
});

test("a_card_in_another_row_does_not_matter", () => {
  expect(bendX(0, 400, 0, 100, [{ x: 150, y: 300, w: 100, h: 100 }])).toBe(200);
});

test("the_ends_own_cards_are_not_obstacles", () => {
  const boxes = [
    { x: -236, y: -20, w: 236, h: 190 },
    { x: 400, y: 160, w: 236, h: 190 },
  ];
  expect(bendX(0, 400, 0, 200, boxes)).toBe(200);
});

test("no_room_at_all_falls_back_to_the_middle", () => {
  expect(bendX(0, 40, 0, 10, [])).toBe(20);
  expect(bendX(0, 400, 0, 200, [{ x: 10, y: 0, w: 380, h: 200 }])).toBe(200);
});
