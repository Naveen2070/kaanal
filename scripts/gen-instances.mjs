#!/usr/bin/env node
// Builds crates/kaanal-core/data/instances.csv (name,vcpu,memory_mib,net_mbps,usd_micro_hr) from the public
// instance catalogues of AWS, Azure and GCP kept by instances.vantage.sh (a mirror of each
// provider's own pages). The csv is what `instance: provider/type` reads (step 8P, RULES 113).
//
// Usage: node --max-old-space-size=8192 scripts/gen-instances.mjs [dir-with-aws.json-azure.json-gcp.json]
// With no argument the three files are downloaded to a temp folder (the AWS one is about 300 MB:
// it carries every price). Run it again when a provider adds families, review the diff, commit.
//
// Estimates, labelled in the csv header:
// - vCPU and memory are the providers' own figures;
// - AWS network is the number in the type's "Up to X Gigabit" / "X Gigabit" text; the old words
//   (Low, Moderate, High) have no figure and give 0 (not limited);
// - GCP's catalogue has no figure; the documented default egress cap of 2 Gbps per vCPU, at most
//   32 Gbps (16 for the E2 family), is used;
// - Azure's catalogue has no figure: the figures Microsoft states on its own size pages are kept by
//   hand in scripts/azure-network.csv (exact, per size); a constrained-vCPU size (E16-8s v5) takes its
//   parent's figure (E16s v5); every other Azure size gives 0 (not limited);
// - price is the on-demand Linux price per hour in micro-USD (1,000,000 = 1 dollar), one default
//   region per provider (AWS us-east-1, Azure East US, GCP us-central1), as the catalogue carries it
//   on the date in the header; 0 = the catalogue has no price for that type in that region. An
//   estimate for a what-if, never a quote.
// 0 means "the source gives none": the node's card is then left unlimited, which is what an absent
// `bandwidth` means.
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
  existsSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

const BASE = "https://instances.vantage.sh";
const SOURCES = { aws: "instances.json", azure: "azure/instances.json", gcp: "gcp/instances.json" };
const dir = process.argv[2] ?? mkdtempSync(join(tmpdir(), "kaanal-instances-"));

function load(provider) {
  const file = join(dir, `${provider}.json`);
  if (!existsSync(file)) {
    console.error(`downloading ${provider} ...`);
    execFileSync("curl", ["-sS", "-L", "-m", "1800", "-o", file, `${BASE}/${SOURCES[provider]}`], {
      stdio: "inherit",
    });
  }
  const j = JSON.parse(readFileSync(file, "utf8"));
  return Array.isArray(j) ? j : Object.values(j);
}

const mib = (gib) => Math.round(Number(gib) * 1024);
const rows = new Map();
// the on-demand Linux price of one region, in micro-USD an hour (0 when there is none)
const micro = (x, region) => {
  const v = Number(x.pricing?.[region]?.linux?.ondemand);
  return Number.isFinite(v) && v > 0 ? Math.round(v * 1e6) : 0;
};
// every region's price of every type: prices[provider][region] = [[name, micro], ...]
const prices = { aws: {}, azure: {}, gcp: {} };
const noteRegions = (provider, name, x) => {
  const n = name.toLowerCase();
  if (!rows.has(n)) return;
  for (const region of Object.keys(x.pricing ?? {})) {
    const m = micro(x, region);
    if (m > 0) (prices[provider][region] ??= []).push([n, m]);
  }
};
const add = (name, vcpu, memoryMib, netMbps, price) => {
  const n = name.toLowerCase();
  if (!(vcpu >= 1) || !(memoryMib >= 1) || rows.has(n)) return;
  rows.set(n, [n, Math.round(vcpu), memoryMib, Math.max(0, Math.round(netMbps)), price]);
};

// AWS: "Up to 12.5 Gigabit", "25 Gigabit", "10 Gigabit"; the rest (Low, Moderate, ...) gives 0.
const awsNet = (text) => {
  const m = /([0-9]+(?:\.[0-9]+)?)\s*Gigabit/i.exec(String(text ?? ""));
  return m ? Number(m[1]) * 1000 : 0;
};
for (const x of load("aws")) {
  add(`aws/${x.instance_type}`, x.vCPU, mib(x.memory), awsNet(x.network_performance), micro(x, "us-east-1"));
  noteRegions("aws", `aws/${x.instance_type}`, x);
}

// Azure: "D2s v5" is Standard_D2s_v5. Network: the hand-kept table, else the parent size's figure
// for a constrained-vCPU size ("E16-8s v5" is an E16s v5 with cores switched off), else 0.
const azureNet = new Map(
  readFileSync(new URL("./azure-network.csv", import.meta.url), "utf8")
    .split("\n")
    .filter((l) => l && !l.startsWith("#"))
    .map((l) => l.split(",")),
);
const constrained = /^(standard_[a-z]+\d+)-\d+([a-z]*(?:_v\d+)?)$/;
const azureNetOf = (name) => {
  const exact = azureNet.get(name);
  if (exact !== undefined) return Number(exact);
  const m = constrained.exec(name);
  return m ? Number(azureNet.get(m[1] + m[2]) ?? 0) : 0;
};
for (const x of load("azure")) {
  const size = String(x.pretty_name_azure ?? x.pretty_name ?? x.instance_type).replace(/\s+/g, "_");
  const name = `standard_${size}`.toLowerCase();
  add(`azure/${name}`, x.vcpu, mib(x.memory), azureNetOf(name), micro(x, "us-east"));
  noteRegions("azure", `azure/${name}`, x);
}

// GCP: 2 Gbps per vCPU up to 32 Gbps (E2: up to 16), the documented default egress cap.
for (const x of load("gcp")) {
  const cap = x.instance_type.startsWith("e2-") ? 16000 : 32000;
  add(`gcp/${x.instance_type}`, x.vCPU, mib(x.memory), Math.min(2000 * Number(x.vCPU), cap), micro(x, "us-central1"));
  noteRegions("gcp", `gcp/${x.instance_type}`, x);
}

const sorted = [...rows.values()].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
const head = [
  "# Cloud instance types for `instance: provider/type` (kaanal-core src/instances.rs).",
  `# Generated ${new Date().toISOString().slice(0, 10)} by scripts/gen-instances.mjs; do not edit by hand.`,
  "# name,vcpu,memory_mib,net_mbps,usd_micro_hr. ESTIMATES: an upper bound on capacity (a vCPU is a hardware thread",
  "# counted as a full core; burst credits are not modelled). net_mbps 0 = the source gives none.",
  "# usd_micro_hr: on-demand Linux price an hour in micro-USD (1000000 = 1 dollar), one default region",
  "# per provider (aws us-east-1, azure us-east, gcp us-central1); 0 = no price. An estimate, not a quote.",
];
writeFileSync(
  new URL("../crates/kaanal-core/data/instances.csv", import.meta.url),
  head.concat(sorted.map((r) => r.join(","))).join("\n") + "\n",
);
// One csv per region (data/prices/<provider>/<region>.csv: name,usd_micro_hr, only types that have a
// price there) and the Rust index that embeds them (src/prices_data.rs), so a region is picked at
// run time without a network call. Rewritten whole each run.
const pricesDir = new URL("../crates/kaanal-core/data/prices/", import.meta.url);
rmSync(pricesDir, { recursive: true, force: true });
const date = new Date().toISOString().slice(0, 10);
const index = [];
for (const provider of Object.keys(prices)) {
  mkdirSync(new URL(`${provider}/`, pricesDir), { recursive: true });
  for (const region of Object.keys(prices[provider]).sort()) {
    const list = prices[provider][region].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
    const body = [
      `# On-demand Linux price an hour in micro-USD (1000000 = 1 dollar) of ${provider} in region ${region}.`,
      `# Generated ${date} by scripts/gen-instances.mjs from instances.vantage.sh; do not edit by hand.`,
      "# name,usd_micro_hr. An estimate for a what-if, not a quote; a type missing here has no price there.",
      ...list.map((r) => r.join(",")),
    ].join("\n");
    writeFileSync(new URL(`${provider}/${region}.csv`, pricesDir), body + "\n");
    index.push([provider, region]);
  }
}
writeFileSync(
  new URL("../crates/kaanal-core/src/prices_data.rs", import.meta.url),
  [
    "//! The per-region price files, embedded. Generated by scripts/gen-instances.mjs; do not edit by hand.",
    "",
    "/// `(provider, region, csv)`, sorted by provider then region.",
    "#[rustfmt::skip]",
    "pub(super) static FILES: &[(&str, &str, &str)] = &[",
    ...index.map(
      ([pr, rg]) => `    ("${pr}", "${rg}", include_str!("../data/prices/${pr}/${rg}.csv")),`,
    ),
    "];",
    "",
  ].join("\n"),
);
console.error(`price files: ${index.length} regions`);
const by = (p) => sorted.filter((r) => r[0].startsWith(`${p}/`)).length;
console.error(`${sorted.length} types: aws ${by("aws")}, azure ${by("azure")}, gcp ${by("gcp")}`);
const have = (p, i) => sorted.filter((r) => r[0].startsWith(`${p}/`) && r[i] > 0).length;
console.error(
  `with a network figure: aws ${have("aws", 3)}, azure ${have("azure", 3)}, gcp ${have("gcp", 3)}; with a price: aws ${have("aws", 4)}, azure ${have("azure", 4)}, gcp ${have("gcp", 4)}`,
);
