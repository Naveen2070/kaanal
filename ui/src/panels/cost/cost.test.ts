import { expect, test } from "vite-plus/test";
import type { Doc } from "../../data/api/types";
import {
  multiAzFactor,
  daysFrom,
  dollarsFor,
  nodeCosts,
  perThousandRequests,
  priceKeysUsed,
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

test("a_managed_database_is_priced_from_its_engines_list_in_the_aws_region", () => {
  const d = doc({
    db: {
      type: "database",
      hosting: "managed",
      engine: "mysql",
      instance: "aws/db.m5.large",
      replicas: 2,
    },
    pg: { type: "database", hosting: "managed", instance: "aws/db.m5.large" },
  });
  expect(priceKeysUsed(d)).toEqual([
    { key: "aws-rds-mysql", provider: "aws" },
    { key: "aws-rds-postgresql", provider: "aws" },
  ]);
  expect(providersUsed(d)).toEqual(["aws"]);
  const tables = {
    "aws-rds-mysql": new Map([["aws/db.m5.large", 171_000]]),
    "aws-rds-postgresql": new Map([["aws/db.m5.large", 178_000]]),
  };
  const c = nodeCosts(d, tables, region);
  expect(c[0]).toMatchObject({ unitMicro: 171_000, totalMicro: 342_000 });
  expect(c[1]).toMatchObject({ unitMicro: 178_000 });
  // the VM price list is never read for a class
  expect(nodeCosts(d, { aws: new Map([["aws/db.m5.large", 1]]) }, region)[0]?.why).toBe(
    "prices are loading",
  );
});

test("serverless_external_and_a_managed_gateway_have_no_machine_to_price_and_say_why", () => {
  const d = doc({
    fn: { type: "service", hosting: "serverless" },
    idp: { type: "auth", hosting: "external" },
    gw: { type: "gateway", hosting: "managed" },
    db: { type: "database", hosting: "managed" },
  });
  const c = nodeCosts(d, {}, region);
  expect(c.map((x) => x.why)).toEqual([
    "serverless: priced per use, not estimated",
    "run by someone else: not estimated",
    "managed: priced by use, not estimated",
    "pick a managed database class",
  ]);
  expect(priceKeysUsed(d)).toEqual([]);
});

test("a_managed_cache_is_priced_from_the_elasticache_list_and_a_serverless_one_is_left_out", () => {
  const d = doc({
    c: { type: "cache", hosting: "managed", instance: "aws/cache.m5.large", replicas: 2 },
    s: { type: "cache", hosting: "serverless" },
    e: { type: "cache", hosting: "managed" },
  });
  expect(priceKeysUsed(d)).toEqual([{ key: "aws-elasticache", provider: "aws" }]);
  const c = nodeCosts(d, { "aws-elasticache": new Map([["aws/cache.m5.large", 156_000]]) }, region);
  expect(c[0]).toMatchObject({ unitMicro: 156_000, totalMicro: 312_000 });
  expect(c[1]?.why).toBe("serverless: priced per use, not estimated");
  expect(c[2]?.why).toBe("pick a managed cache class");
});

test("a_valkey_cache_is_priced_from_the_valkey_list", () => {
  const d = doc({
    v: { type: "cache", flavor: "valkey", hosting: "managed", instance: "aws/cache.m5.large" },
  });
  expect(priceKeysUsed(d)).toEqual([{ key: "aws-elasticache-valkey", provider: "aws" }]);
  const c = nodeCosts(
    d,
    { "aws-elasticache-valkey": new Map([["aws/cache.m5.large", 124_800]]) },
    region,
  );
  expect(c[0]).toMatchObject({ unitMicro: 124_800, totalMicro: 124_800 });
});

test("a_database_flavor_picks_its_engines_list_when_no_engine_is_written", () => {
  const d = doc({
    m: { type: "database", flavor: "mysql", hosting: "managed", instance: "aws/db.m5.large" },
  });
  expect(priceKeysUsed(d)).toEqual([{ key: "aws-rds-mysql", provider: "aws" }]);
});

const GB = 1024 ** 3;
const s3 = { gbMonth: 23_000, writePer1k: 5_000, readPer1k: 400 };

test("an_object_store_costs_its_data_over_a_month_plus_its_requests_at_the_last_runs_rate", () => {
  const d = doc({ files: { type: "storage", flavor: "s3", stored: 500 * GB, replicas: 3 } });
  const prices = { "aws/us-east-1": s3 };
  // 500 GB x $0.023 a GB-month over 730 hours = $0.015753... an hour; replicas do not multiply it
  const only = nodeCosts(d, {}, region, { prices })[0];
  expect(only?.copies).toBe(1);
  expect(only?.totalMicro).toBe(Math.round((500 * 23_000) / 730));
  // 100 visits a second, all reads: 360,000 requests an hour at $0.0004 per 1,000 = $0.144 an hour
  const run = nodeCosts(d, {}, region, { prices, visits: { files: 100 } })[0];
  expect(run?.totalMicro).toBe(Math.round((500 * 23_000) / 730 + 100 * 3600 * (400 / 1000)));
});

test("a_write_share_moves_requests_to_the_write_price_and_a_store_with_nothing_set_says_so", () => {
  const prices = { "aws/us-east-1": s3 };
  const half = doc({ f: { type: "storage", stored: GB, write_share: 0.5 } });
  // 1 visit a second, half writes: 3,600 requests an hour at (0.5 x 5000 + 0.5 x 400) / 1000 micro each
  const c = nodeCosts(half, {}, region, { prices, visits: { f: 1 } })[0];
  expect(c?.totalMicro).toBe(Math.round(23_000 / 730 + 3600 * ((0.5 * 5_000 + 0.5 * 400) / 1000)));
  const none = doc({ f: { type: "storage" } });
  expect(nodeCosts(none, {}, region, { prices })[0]?.why).toMatch(/set stored/);
  expect(nodeCosts(none, {}, region)[0]?.why).toBe("prices are loading");
});

test("a_store_is_priced_in_the_cloud_its_flavor_names_and_the_region_picked", () => {
  const prices = {
    "azure/us-east": { gbMonth: 20_800, writePer1k: 5_000, readPer1k: 400 },
    "gcp/europe-west1": s3,
  };
  const d = doc({
    b: { type: "storage", flavor: "blob", stored: GB },
    g: { type: "storage", flavor: "gcs", stored: GB },
    x: { type: "storage", flavor: "blob", stored: GB },
  });
  expect(providersUsed(d)).toEqual(["aws", "azure", "gcp"].filter((p) => p !== "aws"));
  const c = nodeCosts(d, {}, { azure: "us-east", gcp: "europe-west1" }, { prices });
  expect(c[0]?.totalMicro).toBe(Math.round(20_800 / 730));
  expect(c[1]?.totalMicro).toBe(Math.round(23_000 / 730));
  expect(
    nodeCosts(d, {}, { azure: "nowhere" }, { prices: { "azure/us-east": s3 } })[2]?.why,
  ).toMatch(/no object-store price in nowhere/);
});

test("a_store_location_beats_the_globes_region_and_a_multi_region_is_priced_as_listed", () => {
  const prices = {
    "gcp/us-central1": { gbMonth: 20_000, writePer1k: 5_000, readPer1k: 400 },
    "gcp/nam4": { gbMonth: 44_000, writePer1k: 10_000, readPer1k: 400 },
  };
  const d = doc({
    a: { type: "storage", flavor: "gcs", stored: GB },
    b: { type: "storage", flavor: "gcs", stored: GB, location: "nam4" },
    c: { type: "storage", flavor: "gcs", stored: GB, location: "mars-1" },
  });
  const c = nodeCosts(d, {}, { gcp: "us-central1" }, { prices });
  expect(c[0]?.totalMicro).toBe(Math.round(20_000 / 730));
  expect(c[1]?.totalMicro).toBe(Math.round(44_000 / 730));
  expect(c[2]?.why).toMatch(/no object-store price in mars-1/);
});

test("multi_az_doubles_a_managed_database_but_not_aurora_and_matches_the_rust_figures", () => {
  // the same figures as crates/kaanal-core/tests/cost.rs: db.t3.micro PostgreSQL is $0.018 an hour
  const tables = { "aws-rds-postgresql": new Map([["aws/db.t3.micro", 18_000]]) };
  const d = doc({
    one: { type: "database", flavor: "postgres", hosting: "managed", instance: "aws/db.t3.micro" },
    two: {
      type: "database",
      flavor: "postgres",
      hosting: "managed",
      instance: "aws/db.t3.micro",
      multi_az: true,
    },
  });
  const c = nodeCosts(d, tables, region);
  expect([c[0]?.totalMicro, c[1]?.totalMicro]).toEqual([18_000, 36_000]);
  expect(multiAzFactor({ type: "database", engine: "aurora-mysql", multi_az: true })).toBe(1);
  expect(multiAzFactor({ type: "cache", multi_az: true })).toBe(1);
});
