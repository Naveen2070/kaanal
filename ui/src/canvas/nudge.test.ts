import { expect, test } from "vite-plus/test";
import { nudge } from "./nudge";

test("arrow_keys_nudge_a_selected_node_shift_for_larger_steps", () => {
  expect(nudge([100, 100], "ArrowRight", false)).toEqual([108, 100]);
  expect(nudge([100, 100], "ArrowLeft", false)).toEqual([92, 100]);
  expect(nudge([100, 100], "ArrowUp", true)).toEqual([100, 60]);
  expect(nudge([100, 100], "ArrowDown", true)).toEqual([100, 140]);
  expect(nudge([100, 100], "a", false)).toBeNull();
});
