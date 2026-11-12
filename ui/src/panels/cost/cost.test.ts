import { expect, test } from "vite-plus/test";
import type { Doc } from "../../data/api/types";
import {
  daysFrom,
  dollarsFor,
  nodeCosts,
  perThousandRequests,
  providersUsed,
  totals,
  usd,
} from "./cost";

const doc = (nodes: Record<string, unknown>, instances?: unknown): Doc =>
  ({ nodes, ...(instances ? { instances } : {}) }) as unknown as Doc;
const region = { aws: "us-east-1" };

test("a_node_costs_its_copies_times_the_price_an_hour_in_whole_micro_dollars", () => {
  const d = doc({
    app: { type: "service", instance: "aws/m5.large", replicas: 3 },
    gw: { type: "gateway" },
  });
  const c = nodeCosts(d, { aws: new Map([["aws/m5.large", 96_000]]) }, region);
  expect(c[0]).toMatchObject({ copies: 3, unitMicro: 96_000, totalMicro: 288_000, why: null });
  expect(c[1]).toMatchObject({ unitMicro: null, totalMicro: null, why: "no instance type" });
  expect(totals(c)).toEqual({ perHourMicro: 288_000, priced: 1, unpriced: 1 });
});

test("a_custom_instance_uses_its_own_cost_per_hour_and_says_when_it_has_none", () => {
  const d = doc(
    {
      a: { type: "service", instance: "box", replicas: 2 },
      b: { type: "service", instance: "bare" },
    },
    { box: { cores: 4, cost_per_hour: 0.25 }, bare: { cores: 2 } },
  );
  const c = nodeCosts(d, {}, region);
  expect(c[0]?.totalMicro).toBe(500_000); // 2 x $0.25
  expect(c[1]?.why).toMatch(/no cost_per_hour/);
  expect(providersUsed(d)).toEqual([]); // custom instances name no cloud
});

test("a_provider_still_loading_or_a_type_with_no_price_in_the_region_says_which", () => {
  const d = doc({ a: { type: "service", instance: "aws/m5.large" } });
  expect(nodeCosts(d, {}, region)[0]?.why).toBe("prices are loading");
  expect(nodeCosts(d, { aws: new Map() }, region)[0]?.why).toBe("no price in us-east-1");
  expect(providersUsed(doc({ a: { instance: "azure/x" }, b: { instance: "aws/y" } }))).toEqual([
    "aws",
    "azure",
  ]);
});

test("the_cost_of_days_and_of_a_thousand_requests_follow_from_the_hourly_price", () => {
  // $0.096 an hour for 30 days: 0.096 x 24 x 30 = $69.12
  expect(dollarsFor(96_000, 30)).toBeCloseTo(69.12, 10);
  // $3.60 an hour at 1 request a second is 3,600 requests an hour: $1.00 per 1,000 (hourly / (3600 x rate) x 1000)
  expect(perThousandRequests(3_600_000, 1)).toBeCloseTo(1, 12);
  // ten times the load, a tenth of the cost per request
  expect(perThousandRequests(3_600_000, 10)).toBeCloseTo(0.1, 12);
  expect(perThousandRequests(3_600_000, 0)).toBeNull();
});

test("money_and_days_are_shown_and_read_sensibly", () => {
  expect(usd(1234.4)).toBe("$1,234");
  expect(usd(69.12)).toBe("$69.12");
  expect(usd(0.0034)).toBe("$0.0034");
  expect(usd(0)).toBe("$0");
  expect(daysFrom("45", 30)).toBe(45);
  expect(daysFrom("", 30)).toBe(30);
  expect(daysFrom("0", 30)).toBe(30);
  expect(daysFrom("99999", 30)).toBe(3650);
});
