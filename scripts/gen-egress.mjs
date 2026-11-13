#!/usr/bin/env node
// Builds crates/kaanal-core/data/egress.csv (provider,region,usd_micro_per_gb,free_gb): the first-tier price
// of data sent out to the internet, per GB, in each region (RULES 131, B2). The cost estimate of a run
// multiplies what the run's streams send out by it (kaanal-core src/cost.rs).
//
//   node scripts/gen-egress.mjs [path-to-AWSDataTransfer.json]
//
// - aws: AWS's own price list (AWSDataTransfer, about 55 MB; downloaded to a temp file when no path is
//   given), the "AWS Outbound" to "External" `DataTransfer-Out-Bytes` rows, the tier that begins at 0 GB
//   (0 to 10,240 GB; $0.09 in us-east-1). free_gb 100: AWS's monthly free allowance, across services and
//   regions, a constant (the price list carries it as a separate product).
// - azure: the retail prices API (prices.azure.com), service Bandwidth, product "Rtn Preference: MGN"
//   (Microsoft's global network, the default routing), "Standard Data Transfer Out": the first tier with a
//   price above 0 gives the price and, as its starting GB, the free allowance (100 GB, $0.087).
// - gcp: one row for every region ("*"): the headline Premium Tier price to worldwide destinations,
//   $0.12 a GiB for the first TiB, no allowance. Google's regional pages are rendered by script and
//   read in a browser (see gen-storage-prices.mjs); the headline is the figure the decision recorded.
// An estimate for a what-if, never a quote: later tiers (cheaper above 10 TB), China and Australia
// destinations and CDN or peering routes are not in it.
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { vmName } from "./azure-regions.mjs";

const rows = [];
const micro = (usd) => Math.round(Number(usd) * 1e6);

// ---- AWS
let file = process.argv[2];
if (!file) {
  file = join(mkdtempSync(join(tmpdir(), "kaanal-egress-")), "AWSDataTransfer.json");
}
if (!existsSync(file)) {
  console.error("downloading AWSDataTransfer ...");
  execFileSync("curl", ["-sS", "-L", "-m", "900", "-o", file, "https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AWSDataTransfer/current/index.json"], { stdio: "inherit" });
}
const offer = JSON.parse(readFileSync(file, "utf8"));
const aws = new Map();
for (const p of Object.values(offer.products)) {
  const a = p.attributes;
  if (a.transferType !== "AWS Outbound" || a.toLocation !== "External" || !/^[a-z]{2}(-gov)?-[a-z]+-[0-9]+$/.test(a.fromRegionCode ?? "")) continue; // regions, not local zones or Wavelength
  if (!/(^|-)DataTransfer-Out-Bytes$/.test(a.usagetype ?? "")) continue;
  const dims = Object.values(Object.values(offer.terms.OnDemand[p.sku] ?? {})[0]?.priceDimensions ?? {});
  const first = dims.find((d) => Number(d.beginRange) === 0);
  const v = Number(first?.pricePerUnit?.USD);
  if (v > 0) aws.set(a.fromRegionCode, micro(v));
}
for (const [region, m] of [...aws].sort()) rows.push(["aws", region, m, 100]);

// ---- Azure
const azure = new Map();
let url =
  "https://prices.azure.com/api/retail/prices?api-version=2023-01-01-preview&$filter=" +
  encodeURIComponent("serviceName eq 'Bandwidth' and productName eq 'Rtn Preference: MGN' and meterName eq 'Standard Data Transfer Out' and priceType eq 'Consumption'");
while (url) {
  const j = JSON.parse(execFileSync("curl", ["-sS", "-L", "-m", "120", url], { maxBuffer: 1 << 28 }).toString());
  for (const i of j.Items) {
    if (!(i.retailPrice > 0)) continue;
    const slot = azure.get(i.armRegionName);
    // the first paying tier: the lowest starting GB with a price
    if (!slot || Number(i.tierMinimumUnits) < slot.free) azure.set(i.armRegionName, { price: micro(i.retailPrice), free: Number(i.tierMinimumUnits) });
  }
  url = j.NextPageLink;
}
const left = [];
const seen = new Set();
for (const [arm, s] of [...azure].sort()) {
  const region = vmName(arm);
  if (!region) left.push(arm);
  else if (!seen.has(region)) {
    seen.add(region);
    rows.push(["azure", region, s.price, Math.round(s.free)]);
  }
}
// ---- Google
rows.push(["gcp", "*", 120000, 0]);

writeFileSync(
  new URL("../crates/kaanal-core/data/egress.csv", import.meta.url),
  [
    "# Data sent out to the internet: first-tier price a GB in micro-USD (1000000 = 1 dollar) and the monthly free GB.",
    `# Generated ${new Date().toISOString().slice(0, 10)} by scripts/gen-egress.mjs; do not edit by hand. aws: AWS price list; azure: retail prices API;`,
    "# gcp: the headline price, one row for every region. An estimate for a what-if, not a quote: no later tiers, no CDN.",
    "# provider,region,usd_micro_per_gb,free_gb",
    ...rows.map((r) => r.join(",")),
  ].join("\n") + "\n",
);
console.error(`egress: aws ${aws.size}, azure ${seen.size} regions, gcp 1; azure with no VM-region name (left out): ${left.join(" ") || "none"}`);
