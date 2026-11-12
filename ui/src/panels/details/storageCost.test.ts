import { expect, test } from "vite-plus/test";
import type { NodeDoc } from "../../data/api/types";
import {
  classOf,
  locationLabel,
  locationOf,
  retrievedOf,
  setClass,
  setLocation,
  setRetrieved,
  setStored,
  setWriteShare,
  STORAGE_CLASSES,
  storedOf,
  writeShareOf,
} from "./storageCost";

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

test("the_location_is_written_removed_and_labelled", () => {
  expect(locationOf(node({}))).toBe("");
  expect(locationOf(node({ location: "nam4" }))).toBe("nam4");
  expect(setLocation("s", node({}), "nam4")).toEqual([
    { op: "insert", path: ["nodes", "s", "location"], value: "nam4" },
  ]);
  expect(setLocation("s", node({ location: "nam4" }), "eu")).toEqual([
    { op: "set", path: ["nodes", "s", "location"], value: "eu" },
  ]);
  expect(setLocation("s", node({ location: "nam4" }), "")).toEqual([
    { op: "remove", path: ["nodes", "s", "location"] },
  ]);
  expect(setLocation("s", node({}), "")).toEqual([]);
  expect(locationLabel("nam4")).toBe("nam4 (dual-region)");
  expect(locationLabel("eu")).toBe("eu (multi-region)");
  expect(locationLabel("us-east-1")).toBe("us-east-1");
});

test("the_class_is_written_removed_at_standard_and_the_data_read_back_is_a_size", () => {
  expect(classOf(node({}))).toBe("standard");
  expect(classOf(node({ class: "cold" }))).toBe("cold");
  expect(setClass("s", node({}), "infrequent")).toEqual([
    { op: "insert", path: ["nodes", "s", "class"], value: "infrequent" },
  ]);
  expect(setClass("s", node({ class: "cold" }), "archive")).toEqual([
    { op: "set", path: ["nodes", "s", "class"], value: "archive" },
  ]);
  expect(setClass("s", node({ class: "cold" }), "standard")).toEqual([
    { op: "remove", path: ["nodes", "s", "class"] },
  ]);
  expect(setClass("s", node({}), "standard")).toEqual([]);
  expect(STORAGE_CLASSES.map((c) => c.id)).toEqual(["standard", "infrequent", "cold", "archive"]);
  expect(retrievedOf(node({ retrieved: "50GB" }))).toBe(50 * 1024 ** 3);
  expect(setRetrieved("s", node({}), 1024)).toEqual([
    { op: "insert", path: ["nodes", "s", "retrieved"], value: 1024 },
  ]);
  expect(setRetrieved("s", node({ retrieved: "1GB" }), 0)).toEqual([
    { op: "remove", path: ["nodes", "s", "retrieved"] },
  ]);
});
