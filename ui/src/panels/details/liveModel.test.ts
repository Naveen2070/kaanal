import { expect, test } from "vite-plus/test";
import { describePin, leaseLeft, pinBody, statusRange, takesDelay } from "./liveModel";

test("only_error_and_empty_take_a_status_and_only_success_takes_no_delay", () => {
  expect(statusRange("error")).toEqual({ min: 400, max: 599 });
  expect(statusRange("empty")).toEqual({ min: 200, max: 299 });
  expect(statusRange("loading")).toBeNull();
  expect(statusRange("slow")).toBeNull();
  expect(statusRange("success")).toBeNull();
  expect(takesDelay("success")).toBe(false);
  expect(takesDelay("slow")).toBe(true);
});

test("a_pin_body_carries_only_what_the_state_allows", () => {
  expect(pinBody("GET /orders", "error", "429", "500")).toEqual({
    body: { endpoint: "GET /orders", state: "error", status: 429, delay: "500ms" },
  });
  // empty boxes mean the state's own defaults
  expect(pinBody("GET /orders", "error", "", "")).toEqual({
    body: { endpoint: "GET /orders", state: "error" },
  });
  // a status in a box the state does not take is dropped, not sent (the API would answer 400)
  expect(pinBody("GET /a", "slow", "500", "2000")).toEqual({
    body: { endpoint: "GET /a", state: "slow", delay: "2000ms" },
  });
  expect(pinBody("GET /a", "success", "200", "5")).toEqual({
    body: { endpoint: "GET /a", state: "success" },
  });
});

test("what_the_api_would_refuse_is_said_in_words_and_not_sent", () => {
  expect(pinBody("", "error", "", "")).toEqual({ problem: "Pick an endpoint." });
  expect(pinBody("GET /a", "error", "200", "")).toMatchObject({
    problem: expect.stringMatching(/400 to 599/),
  });
  expect(pinBody("GET /a", "error", "40.5", "")).toMatchObject({
    problem: expect.stringMatching(/whole number/),
  });
  expect(pinBody("GET /a", "empty", "404", "")).toMatchObject({
    problem: expect.stringMatching(/200 to 299/),
  });
  expect(pinBody("GET /a", "slow", "", "0")).toMatchObject({
    problem: expect.stringMatching(/above 0/),
  });
  expect(pinBody("GET /a", "slow", "", "300001")).toMatchObject({
    problem: expect.stringMatching(/300,000/),
  });
  expect(pinBody("GET /a", "slow", "", "abc")).toMatchObject({
    problem: expect.stringMatching(/above 0/),
  });
});

test("pins_and_leases_read_as_short_sentences", () => {
  expect(describePin({ state: "error", status: 429, delay_ms: 500 })).toBe(
    "error 429, after 500 ms",
  );
  expect(describePin({ state: "loading" })).toBe("loading");
  expect(leaseLeft(29_100)).toBe("30 s left");
  expect(leaseLeft(119_000)).toBe("119 s left");
  expect(leaseLeft(240_000)).toBe("4 min left");
});
