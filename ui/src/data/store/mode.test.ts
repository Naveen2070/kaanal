import { expect, test } from "vite-plus/test";
import { isLive, useMode } from "./mode";

test("admin_shares_the_live_view_and_batch_does_not", () => {
  expect(isLive("live")).toBe(true);
  expect(isLive("admin")).toBe(true);
  expect(isLive("batch")).toBe(false);
});

test("studio_opens_live_and_the_mode_can_be_set_to_admin", () => {
  expect(useMode.getState().mode).toBe("live");
  useMode.getState().set("admin");
  expect(useMode.getState().mode).toBe("admin");
  useMode.getState().set("live");
});
