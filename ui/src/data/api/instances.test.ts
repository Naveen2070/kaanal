import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import { TOKEN_KEY } from "../../token";
import { mockFetch } from "../../test/fetch";
import { findInstance, instanceLine, loadInstances, matching, resetInstances } from "./instances";

const TABLE = {
  types: [
    ["aws/c6i.large", 2, 4096, 12500],
    ["aws/m5.large", 2, 8192, 10000],
    ["azure/standard_d2s_v5", 2, 8192, 0],
    ["gcp/n2-standard-2", 2, 8192, 10000],
  ],
};

beforeEach(() => {
  sessionStorage.setItem(TOKEN_KEY, "tok");
  resetInstances();
});
afterEach(() => sessionStorage.clear());

test("the_table_is_fetched_once_and_read_into_rows", async () => {
  const sent = mockFetch([{ json: TABLE }]);
  const rows = await loadInstances();
  expect(await loadInstances()).toBe(rows);
  expect(sent).toHaveLength(1);
  expect(sent[0]?.url).toBe("/__kaanal/instances");
  expect(rows[1]).toEqual({ name: "aws/m5.large", vcpu: 2, memMib: 8192, netMbps: 10000 });
});

test("a_failed_fetch_is_tried_again_next_time", async () => {
  mockFetch([{ status: 500, json: { error: "x" } }, { json: TABLE }]);
  await expect(loadInstances()).rejects.toBeTruthy();
  expect((await loadInstances()).length).toBe(4);
});

test("names_are_found_whatever_the_case_and_described_in_one_line", async () => {
  mockFetch([{ json: TABLE }]);
  const rows = await loadInstances();
  const m5 = findInstance(rows, " AWS/M5.Large ");
  expect(m5 && instanceLine(m5)).toBe("2 vCPU, 8 GiB, up to 10 Gbit/s");
  expect(findInstance(rows, "aws/m5.huge")).toBeUndefined();
  const azure = findInstance(rows, "azure/standard_d2s_v5");
  expect(azure && instanceLine(azure)).toBe("2 vCPU, 8 GiB, network not limited");
});

test("a_search_lists_only_that_providers_types_that_contain_the_text", async () => {
  mockFetch([{ json: TABLE }]);
  const rows = await loadInstances();
  expect(matching(rows, "aws", "large").map((r) => r.name)).toEqual([
    "aws/c6i.large",
    "aws/m5.large",
  ]);
  expect(matching(rows, "aws", "aws/m5").map((r) => r.name)).toEqual(["aws/m5.large"]);
  expect(matching(rows, "gcp", "").length).toBe(1);
  expect(matching(rows, "aws", "", 1).length).toBe(1);
});
