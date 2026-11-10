#!/usr/bin/env node
// Builds crates/kaanal-core/data/instances.csv (name,vcpu,memory_mib,net_mbps) from the public
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
// - Azure's catalogue has no figure: 0 (not limited).
// 0 means "the source gives none": the node's card is then left unlimited, which is what an absent
// `bandwidth` means.
import { mkdtempSync, readFileSync, writeFileSync, existsSync } from "node:fs";
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
const add = (name, vcpu, memoryMib, netMbps) => {
  const n = name.toLowerCase();
  if (!(vcpu >= 1) || !(memoryMib >= 1) || rows.has(n)) return;
  rows.set(n, [n, Math.round(vcpu), memoryMib, Math.max(0, Math.round(netMbps))]);
};

// AWS: "Up to 12.5 Gigabit", "25 Gigabit", "10 Gigabit"; the rest (Low, Moderate, ...) gives 0.
const awsNet = (text) => {
  const m = /([0-9]+(?:\.[0-9]+)?)\s*Gigabit/i.exec(String(text ?? ""));
  return m ? Number(m[1]) * 1000 : 0;
};
for (const x of load("aws")) {
  add(`aws/${x.instance_type}`, x.vCPU, mib(x.memory), awsNet(x.network_performance));
}

// Azure: "D2s v5" is Standard_D2s_v5.
for (const x of load("azure")) {
  const size = String(x.pretty_name_azure ?? x.pretty_name ?? x.instance_type).replace(/\s+/g, "_");
  add(`azure/standard_${size}`, x.vcpu, mib(x.memory), 0);
}

// GCP: 2 Gbps per vCPU up to 32 Gbps (E2: up to 16), the documented default egress cap.
for (const x of load("gcp")) {
  const cap = x.instance_type.startsWith("e2-") ? 16000 : 32000;
  add(`gcp/${x.instance_type}`, x.vCPU, mib(x.memory), Math.min(2000 * Number(x.vCPU), cap));
}

const sorted = [...rows.values()].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
const head = [
  "# Cloud instance types for `instance: provider/type` (kaanal-core src/instances.rs).",
  `# Generated ${new Date().toISOString().slice(0, 10)} by scripts/gen-instances.mjs; do not edit by hand.`,
  "# name,vcpu,memory_mib,net_mbps. ESTIMATES: an upper bound on capacity (a vCPU is a hardware thread",
  "# counted as a full core; burst credits are not modelled). net_mbps 0 = the source gives none.",
];
writeFileSync(
  new URL("../crates/kaanal-core/data/instances.csv", import.meta.url),
  head.concat(sorted.map((r) => r.join(","))).join("\n") + "\n",
);
const by = (p) => sorted.filter((r) => r[0].startsWith(`${p}/`)).length;
console.error(`${sorted.length} types: aws ${by("aws")}, azure ${by("azure")}, gcp ${by("gcp")}`);
