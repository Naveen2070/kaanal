#!/usr/bin/env node
// Builds crates/kaanal-core/data/per-use.csv (service,region,usd_micro_per_unit): the published prices of
// the pay-per-use services the cost estimate prices from a run's visits (RULES 131, B11). AWS's own price
// lists (AWSLambda, AmazonApiGateway), the first tier of each, x86 for Lambda:
//   lambda_request   micro-USD per million invocations                 ($0.20 in us-east-1)
//   lambda_gb_second micro-USD per 1,000 GB-seconds of billed duration ($0.0000166667 a GB-s in us-east-1)
//   apigw_request    micro-USD per million REST API requests, first 333 M ($3.50 in us-east-1; an HTTP API is
//                    $1.00 and is not priced)
// No free tier is subtracted. Usage: node scripts/gen-per-use.mjs [folder holding AWSLambda.json and AmazonApiGateway.json]
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const dir = process.argv[2] ?? mkdtempSync(join(tmpdir(), "kaanal-peruse-"));
function offer(name) {
  const file = join(dir, `${name}.json`);
  if (!existsSync(file)) {
    console.error(`downloading ${name} ...`);
    execFileSync("curl", ["-sS", "-L", "-m", "600", "-o", file, `https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/${name}/current/index.json`], { stdio: "inherit" });
  }
  return JSON.parse(readFileSync(file, "utf8"));
}
const rows = [];
// the first tier of one product: the dimension that begins at 0; `scale` turns USD a unit into micro-USD a `per` of them
function collect(name, service, usagetype, scale) {
  const o = offer(name);
  for (const p of Object.values(o.products)) {
    const a = p.attributes;
    if (!usagetype.test(a.usagetype ?? "") || !/^[a-z]{2}(-gov)?-[a-z]+-[0-9]+$/.test(a.regionCode ?? "")) continue;
    const dims = Object.values(Object.values(o.terms.OnDemand[p.sku] ?? {})[0]?.priceDimensions ?? {});
    const v = Number(dims.find((d) => Number(d.beginRange) === 0)?.pricePerUnit?.USD);
    if (v > 0) rows.push([service, a.regionCode, Math.round(v * scale)]);
  }
}
collect("AWSLambda", "lambda_request", /^([A-Z0-9]+-)?Request$/, 1e12); // USD a request -> micro-USD a million
collect("AWSLambda", "lambda_gb_second", /^([A-Z0-9]+-)?Lambda-GB-Second$/, 1e9); // USD a GB-s -> micro-USD a 1,000 GB-s
collect("AmazonApiGateway", "apigw_request", /^([A-Z0-9]+-)?ApiGatewayRequest$/, 1e12);
rows.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : a[1] < b[1] ? -1 : a[1] > b[1] ? 1 : 0));
writeFileSync(
  new URL("../crates/kaanal-core/data/per-use.csv", import.meta.url),
  [
    "# Pay-per-use prices, first tier, in micro-USD (1000000 = 1 dollar): lambda_request and apigw_request a million requests,",
    `# lambda_gb_second a thousand GB-seconds. Generated ${new Date().toISOString().slice(0, 10)} by scripts/gen-per-use.mjs from AWS's price lists; do not edit by hand.`,
    "# No free tier subtracted; an estimate for a what-if, not a quote.",
    "# service,region,usd_micro_per_unit",
    ...rows.map((r) => r.join(",")),
  ].join("\n") + "\n",
);
const n = (s) => rows.filter((r) => r[0] === s).length;
console.error(`per-use: lambda_request ${n("lambda_request")}, lambda_gb_second ${n("lambda_gb_second")}, apigw_request ${n("apigw_request")} regions`);
