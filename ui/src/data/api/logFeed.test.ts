import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { TOKEN_KEY } from "../../token";
import { mockFetch } from "../../test/fetch";
import { startFeed, type FeedRow } from "./logFeed";

const row = (done: number): FeedRow => ({
  id: `r-${done}`,
  done,
  ts_ms: 1000 + done,
  method: "GET",
  path: "/users/1",
  endpoint: "GET /users/{id}",
  status: 200,
  ms: 3,
  edited: false,
  pin: null,
  rejected: null,
});

beforeEach(() => {
  sessionStorage.setItem(TOKEN_KEY, "tok");
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  sessionStorage.clear();
});

test("it_asks_for_rows_after_the_last_one_it_saw", async () => {
  const sent = mockFetch([
    { json: { entries: [row(1), row(2)] } },
    { json: { entries: [row(3)] } },
  ]);
  const got: FeedRow[][] = [];
  const stop = startFeed((rows) => got.push(rows), { everyMs: 1000 });
  await vi.advanceTimersByTimeAsync(0);
  await vi.advanceTimersByTimeAsync(1000);
  stop();
  expect(sent[0]?.url).toBe("/__kaanal/logs?limit=100");
  expect(sent[1]?.url).toBe("/__kaanal/logs?after=2&limit=200");
  expect(got.map((r) => r.map((x) => x.done))).toEqual([[1, 2], [3]]);
});

test("it_does_not_poll_while_the_page_is_hidden_and_stops_when_told", async () => {
  const sent = mockFetch([{ json: { entries: [] } }]);
  const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
  const stop = startFeed(() => {}, { everyMs: 1000 });
  await vi.advanceTimersByTimeAsync(3000);
  expect(sent).toHaveLength(0);
  hidden.mockReturnValue(false);
  await vi.advanceTimersByTimeAsync(1000);
  expect(sent.length).toBeGreaterThan(0);
  stop();
  const n = sent.length;
  await vi.advanceTimersByTimeAsync(5000);
  expect(sent).toHaveLength(n);
});

test("a_failed_poll_is_skipped_and_the_next_one_goes_on", async () => {
  const sent = mockFetch([{ status: 500, json: { error: "x" } }, { json: { entries: [row(1)] } }]);
  const got: FeedRow[][] = [];
  const stop = startFeed((rows) => got.push(rows), { everyMs: 1000 });
  await vi.advanceTimersByTimeAsync(1000);
  stop();
  expect(sent.length).toBeGreaterThanOrEqual(2);
  expect(got.flat().map((r) => r.done)).toEqual([1]);
});
