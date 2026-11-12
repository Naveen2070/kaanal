#!/usr/bin/env node
// Builds crates/kaanal-core/data/storage-prices.csv: what an object store costs per region, for the
// cost estimate of a `storage` node (RULES 123).
//
// Usage: node --max-old-space-size=4096 scripts/gen-storage-prices.mjs [--gcp-only] [dir-with-s3.json]
// --gcp-only redoes the Google rows from scripts/gcs-storage.csv and keeps every aws and azure row of the
// file as it is (no download).
//
// - AWS S3 Standard, from AWS's own public price list (pricing.us-east-1.amazonaws.com, AmazonS3):
//   storage per GB-month (the first tier, up to 50 TB), PUT/COPY/POST/LIST (request tier 1) and
//   GET/SELECT (request tier 2) per request. Every region the list prices.
// - Azure Blob, from Azure's public retail prices API (prices.azure.com): General Block Blob v2,
//   Hot, LRS: data stored per GB-month (first tier), write and read operations per 10,000. Every
//   region it prices.
// - Google Cloud Storage: Standard, single regions, from scripts/gcs-storage.csv, kept by hand from
//   https://cloud.google.com/storage/pricing (read in a browser on 2026-11-12: the page draws its
//   tables with script, and the Cloud Billing catalogue needs an API key). The page gives a price per
//   gibibyte hour; the month is that x 730. Class A (writes) $0.005 and Class B (reads) $0.0004 per
//   1,000 in every single region.
//
// Units in the csv: micro-USD (1,000,000 = 1 dollar), integers, so sums are exact:
//   provider,region,gb_month,write_per_1k,read_per_1k
// Estimates for a what-if: Standard class, first tier, no data transfer, no retrieval fees, no
// lifecycle or replication; never a quote.
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const GCP_ONLY = process.argv.includes("--gcp-only");
const dir = process.argv.slice(2).find((a) => !a.startsWith("--")) ?? mkdtempSync(join(tmpdir(), "kaanal-storage-"));
const micro = (usd) => Math.round(Number(usd) * 1e6);
const rows = [];

const csvUrl = new URL("../crates/kaanal-core/data/storage-prices.csv", import.meta.url);
if (GCP_ONLY) {
  for (const l of readFileSync(csvUrl, "utf8").split(/\r?\n/)) {
    if (l && !l.startsWith("#") && !l.startsWith("gcp,")) rows.push(l.split(",").map((f, i) => (i < 2 ? f : Number(f))));
  }
}
if (!GCP_ONLY) {
// ---- AWS S3
const s3file = join(dir, "s3.json");
if (!existsSync(s3file)) {
  console.error("downloading the S3 price list ...");
  execFileSync(
    "curl",
    ["-sS", "-L", "-m", "900", "-o", s3file, "https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonS3/current/index.json"],
    { stdio: "inherit" },
  );
}
const s3 = JSON.parse(readFileSync(s3file, "utf8"));
const first = (p) => {
  const term = Object.values(s3.terms.OnDemand[p.sku] ?? {})[0];
  const dims = Object.values(term?.priceDimensions ?? {}).sort((a, b) => Number(a.beginRange) - Number(b.beginRange));
  return dims[0] ? Number(dims[0].pricePerUnit.USD) : NaN;
};
const aws = {};
for (const p of Object.values(s3.products)) {
  const a = p.attributes;
  if (!a.regionCode || !a.usagetype) continue;
  const slot = (aws[a.regionCode] ??= {});
  // the region's usagetype carries a prefix (EUW1-TimedStorage-ByteHrs); us-east-1 has none
  if (/(^|-)TimedStorage-ByteHrs$/.test(a.usagetype) && a.storageClass === "General Purpose" && a.volumeType === "Standard") {
    slot.gb ??= first(p);
  } else if (/(^|-)Requests-Tier1$/.test(a.usagetype)) slot.put ??= first(p);
  else if (/(^|-)Requests-Tier2$/.test(a.usagetype)) slot.get ??= first(p);
}
for (const [region, s] of Object.entries(aws).sort()) {
  if (![s.gb, s.put, s.get].every((v) => Number.isFinite(v) && v > 0)) continue;
  rows.push(["aws", region, micro(s.gb), micro(s.put * 1000), micro(s.get * 1000)]);
}
const useast = aws["us-east-1"];

// ---- Azure Blob
const azure = {};
let url =
  "https://prices.azure.com/api/retail/prices?$filter=" +
  encodeURIComponent("serviceName eq 'Storage' and productName eq 'General Block Blob v2' and skuName eq 'Hot LRS' and priceType eq 'Consumption'");
while (url) {
  const j = JSON.parse(execFileSync("curl", ["-sS", "-L", "-m", "120", url], { maxBuffer: 1 << 28 }).toString());
  for (const i of j.Items) {
    const slot = (azure[i.armRegionName] ??= {});
    if (i.meterName === "Hot LRS Data Stored" && Number(i.tierMinimumUnits) === 0) slot.gb = i.retailPrice;
    else if (i.meterName === "Hot LRS Write Operations") slot.put = i.retailPrice / 10; // per 10K -> per 1K
    else if (i.meterName === "Hot Read Operations") slot.get = i.retailPrice / 10;
  }
  url = j.NextPageLink;
}
// The API names a region `eastus`; the VM price files (and Studio's region menu) use `us-east`. The
// two are joined by trying the readable spellings of the API name against the VM region list; a
// region with no match is reported and left out (nothing is guessed).
const vmRegions = new Set(
  readdirSync(new URL("../crates/kaanal-core/data/prices/azure/", import.meta.url)).map((f) => f.replace(/.csv$/, "")),
);
const DIRS = ["southcentral", "northcentral", "westcentral", "southeast", "northeast", "southwest", "northwest", "central", "east", "west", "north", "south"];
const ALIAS = { asia: ["asia-pacific"], southafrica: ["south-africa"], newzealand: ["new-zealand"] };
const hyphen = (d) => d.replace(/^(south|north|west)(central)$/, "$1-$2");
function vmName(arm) {
  // a trailing number is part of the VM name too (eastus2 is us-east-2)
  const num = /^(.*?)([0-9]+)$/.exec(arm);
  if (num) {
    const base = vmName(num[1]);
    return base && vmRegions.has(`${base}-${num[2]}`) ? `${base}-${num[2]}` : null;
  }
  for (const d of DIRS) {
    for (const geo of [arm.startsWith(d) ? arm.slice(d.length) : null, arm.endsWith(d) ? arm.slice(0, -d.length) : null]) {
      if (!geo) continue;
      for (const g of ALIAS[geo] ?? [geo]) {
        for (const c of [`${g}-${hyphen(d)}`, `${hyphen(d)}-${g}`]) if (vmRegions.has(c)) return c;
      }
    }
  }
  return null;
}
const unmatched = [];
for (const [arm, s] of Object.entries(azure).sort()) {
  if (![s.gb, s.put, s.get].every((v) => Number.isFinite(v) && v > 0)) continue;
  const region = vmName(arm);
  if (!region) {
    unmatched.push(arm);
    continue;
  }
  // put and get are already per 1,000 operations
  rows.push(["azure", region, micro(s.gb), micro(s.put), micro(s.get)]);
}
console.error(`azure regions with no VM-list name (left out): ${unmatched.join(" ") || "none"}`);

}

// ---- Google Cloud Storage: the hand-kept page figures
for (const line of readFileSync(new URL("./gcs-storage.csv", import.meta.url), "utf8").split("\n")) {
  if (!line || line.startsWith("#")) continue;
  const [region, perHour, wide] = line.split(",");
  // a dual-region or multi-region ("wide") bucket pays $0.01 per 1,000 Class A operations, not $0.005
  rows.push(["gcp", region, Math.round(Number(perHour) * 730 * 1e6), micro(wide?.trim() === "wide" ? 0.01 : 0.005), micro(0.0004)]);
}

const head = [
  "# Object-store prices for the cost estimate of a `storage` node (kaanal-core src/prices.rs, RULES 123).",
  `# Generated ${new Date().toISOString().slice(0, 10)} by scripts/gen-storage-prices.mjs; do not edit by hand.`,
  "# provider,region,gb_month,write_per_1k,read_per_1k in micro-USD (1000000 = 1 dollar). Standard class, first tier.",
  "# aws: S3 (AWS price list); azure: Blob, Hot LRS (Azure retail prices API); gcp: Cloud Storage Standard, single",
  "# regions and the predefined dual- and multi-regions (scripts/gcs-storage.csv, read from cloud.google.com/storage/pricing).",
  "# ESTIMATES, not quotes.",
];
rows.sort((a, b) => (a[0] + "/" + a[1] < b[0] + "/" + b[1] ? -1 : 1));
writeFileSync(
  csvUrl,
  head.concat(rows.map((r) => r.join(","))).join("\n") + "\n",
);
console.error(`${rows.length} rows: aws ${rows.filter((r) => r[0] === "aws").length}, azure ${rows.filter((r) => r[0] === "azure").length}, gcp ${rows.filter((r) => r[0] === "gcp").length}`);
