import { expect, test } from "vite-plus/test";
import type { LogLine } from "../../../data/api/types";
import { groupRequests, sampleFinding } from "./groups";

function line(over: Partial<LogLine> = {}): LogLine {
  return {
    t_ms: 0,
    level: "info",
    request: 1,
    stream: "GET /",
    node: "api",
    replica: 0,
    event: "served",
    wait_ms: 0,
    service_ms: 10,
    status: null,
    note: "proc",
    ...over,
  };
}
const fail = (over: Partial<LogLine> = {}) =>
  line({
    level: "error",
    event: "overflowed",
    wait_ms: null,
    service_ms: null,
    status: 503,
    note: "queue full",
    ...over,
  });

test("lines of one request group together in time order, whichever page they came on", () => {
  const first = [line({ request: 1, t_ms: 20, node: "db" }), line({ request: 2, t_ms: 5 })];
  const second = [line({ request: 1, t_ms: 0, node: "gw" })];
  const g = groupRequests("s1", [...first, ...second]);
  expect(g.map((x) => x.request)).toEqual([1, 2]);
  expect(g[0]?.lines.map((l) => l.node)).toEqual(["gw", "db"]);
  expect(g[0]?.total_ms).toBe(30);
});

test("the same request number on two endpoints is two requests", () => {
  const g = groupRequests("s1", [line({ stream: "GET /a" }), line({ stream: "GET /b" })]);
  expect(g).toHaveLength(2);
});

test("a header says who made the request wait, or nothing when none did", () => {
  const [waited] = groupRequests("s1", [
    line({ event: "queued", wait_ms: 9300, node: "api" }),
    line({ t_ms: 9310, node: "db", wait_ms: 20 }),
  ]);
  expect(waited?.waited).toBe("Waited 9.3 s at api");
  const [calm] = groupRequests("s1", [line()]);
  expect(calm?.waited).toBeNull();
  expect(calm?.failure).toBeNull();
});

test("a failed request carries its cause in words and no invented total", () => {
  const [g] = groupRequests("s1", [fail({ node: "db" })]);
  expect(g?.failure).toBe("Queue full");
  expect(g?.total_ms).toBeNull();
});

test("the one finding: failures first, then the worst wait, then that nothing waited", () => {
  expect(sampleFinding([])).toBeNull();
  const wait = groupRequests("s1", [
    line({ request: 1, event: "queued", wait_ms: 9300 }),
    line({ request: 2, event: "queued", wait_ms: 40, node: "db" }),
  ]);
  expect(sampleFinding(wait)).toBe("api is where requests waited: 9.3 s at worst");
  expect(sampleFinding(groupRequests("s1", [line()]))).toBe("No request waited");
  const failed = groupRequests("s1", [
    fail({ request: 1, node: "db" }),
    fail({ request: 2, node: "db" }),
    line({ request: 3 }),
  ]);
  expect(sampleFinding(failed)).toBe("2 requests failed, all at db");
  const mixed = groupRequests("s1", [
    fail({ request: 1, node: "db" }),
    fail({ request: 2, node: "db" }),
    fail({ request: 3, node: "api" }),
  ]);
  expect(sampleFinding(mixed)).toBe("3 requests failed, most at db");
  expect(sampleFinding(groupRequests("s1", [fail()]))).toBe("1 request failed, all at api");
});
