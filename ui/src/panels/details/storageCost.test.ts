import { expect, test } from "vite-plus/test";
import type { NodeDoc } from "../../data/api/types";
import { setStored, setWriteShare, storedOf, writeShareOf } from "./storageCost";

const node = (extra: Record<string, unknown>): NodeDoc =>
  ({ type: "storage", ...extra }) as NodeDoc;

test("the_data_kept_reads_a_size_and_writes_whole_bytes_and_zero_removes_it", () => {
  expect(storedOf(node({ stored: "500GB" }))).toBe(500 * 1024 ** 3);
  expect(storedOf(node({}))).toBeNull();
  expect(setStored("s", node({}), 1024)).toEqual([
    { op: "insert", path: ["nodes", "s", "stored"], value: 1024 },
  ]);
  expect(setStored("s", node({ stored: "1GB" }), 2048)).toEqual([
    { op: "set", path: ["nodes", "s", "stored"], value: 2048 },
  ]);
  expect(setStored("s", node({ stored: "1GB" }), 0)).toEqual([
    { op: "remove", path: ["nodes", "s", "stored"] },
  ]);
  expect(setStored("s", node({}), 0)).toEqual([]);
});

test("the_write_share_is_kept_between_0_and_1_and_zero_is_the_default", () => {
  expect(writeShareOf(node({ write_share: 0.25 }))).toBe(0.25);
  expect(writeShareOf(node({ write_share: 3 }))).toBeNull();
  expect(setWriteShare("s", node({}), 2)).toEqual([
    { op: "insert", path: ["nodes", "s", "write_share"], value: 1 },
  ]);
  expect(setWriteShare("s", node({ write_share: 0.5 }), 0)).toEqual([
    { op: "remove", path: ["nodes", "s", "write_share"] },
  ]);
});
