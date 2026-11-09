import { beforeEach, expect, test } from "vite-plus/test";
import type { FeedRow } from "../api/logFeed";
import { MAX_ROWS, useAdmin } from "./admin";

const row = (done: number, over: Partial<FeedRow> = {}): FeedRow => ({
  id: `r-${done}`,
  done,
  ts_ms: done,
  method: "GET",
  path: "/x",
  endpoint: "GET /x",
  status: 200,
  ms: 1,
  ...over,
});

beforeEach(() => useAdmin.getState().reset());

test("rows_are_kept_in_order_and_capped_dropping_the_oldest", () => {
  useAdmin.getState().addRows(Array.from({ length: MAX_ROWS + 5 }, (_, i) => row(i + 1)));
  const rows = useAdmin.getState().rows;
  expect(rows).toHaveLength(MAX_ROWS);
  expect(rows[0]?.done).toBe(6);
  expect(rows.at(-1)?.done).toBe(MAX_ROWS + 5);
});

test("an_empty_batch_changes_nothing", () => {
  useAdmin.getState().addRows([row(1)]);
  const before = useAdmin.getState().rows;
  useAdmin.getState().addRows([]);
  expect(useAdmin.getState().rows).toBe(before);
});

test("real_only_is_off_by_default_and_toggles", () => {
  expect(useAdmin.getState().realOnly).toBe(false);
  useAdmin.getState().setRealOnly(true);
  expect(useAdmin.getState().realOnly).toBe(true);
});

test("the_same_requests_added_twice_show_once", () => {
  useAdmin.getState().addRows([row(1), row(2)]);
  // a feed that restarts reads the newest rows again
  useAdmin.getState().addRows([row(1), row(2), row(3)]);
  expect(useAdmin.getState().rows.map((r) => r.id)).toEqual(["r-1", "r-2", "r-3"]);
});
