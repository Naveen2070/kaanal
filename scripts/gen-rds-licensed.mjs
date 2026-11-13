#!/usr/bin/env node
// Licence-included Oracle and SQL Server on Amazon RDS (RULES 131, B15): one price folder per edition,
// `aws-rds-<engine>/<region>.csv` (name,usd_micro_hr), from AWS's own per-region price files
// (AmazonRDS/current/<region>/index.json, about 30 MB each, fetched one at a time and deleted).
//
//   node --max-old-space-size=4096 scripts/gen-rds-licensed.mjs
//
// The editions are the ones AWS lists as "License included" for the database engines "Oracle" and "SQL Server":
//   oracle-se2 (Standard Two), sqlserver-ex (Express), sqlserver-web, sqlserver-standard, sqlserver-enterprise.
// Left out, and why: bring-your-own-license Oracle and bring-your-own-media SQL Server (the price is the plain
// instance price, the licence is yours); Oracle Enterprise, whose BYOL class has two different prices under two
// product codes in the same file; the Outposts rows. Single-AZ, on-demand, instance hours only. A class that
// is not already in instances.csv is reported and left out (no size is invented). The instance price is
// the licence-included price: the licence is inside it.
// Also checks the Multi-AZ rule of cost.rs (a standby is a second instance, so exactly 2 x) on every class and
// region it reads, and prints the range.
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const data = new URL("../crates/kaanal-core/data/", import.meta.url);
const EDITIONS = {
  "Oracle|Standard Two": "oracle-se2",
  "SQL Server|Express": "sqlserver-ex",
  "SQL Server|Web": "sqlserver-web",
  "SQL Server|Standard": "sqlserver-standard",
  "SQL Server|Enterprise": "sqlserver-enterprise",
};
const known = new Set(
  readFileSync(new URL("instances.csv", data), "utf8").split("\n").filter((l) => l && !l.startsWith("#")).map((l) => l.split(",")[0]),
);
const regions = readdirSync(new URL("prices/aws-rds-postgresql/", data)).map((f) => f.replace(/\.csv$/, "")).sort();
const tmp = mkdtempSync(join(tmpdir(), "kaanal-rds-"));
const out = {}; // engine -> region -> [[name, micro]]
const left = new Set();
let ratioMin = Infinity;
let ratioMax = 0;
let pairs = 0;
let nearTwo = 0;
for (const region of regions) {
  const file = join(tmp, "rds.json");
  execFileSync("curl", ["-sS", "-L", "-m", "900", "-o", file, `https://pricing.us-east-1.amazonaws.com/offers/v1.0/aws/AmazonRDS/current/${region}/index.json`], { stdio: "inherit" });
  let o;
  try {
    o = JSON.parse(readFileSync(file, "utf8"));
  } catch {
    console.error(`${region}: no usable price file, skipped`);
    continue;
  }
  const usd = (p) => Number(Object.values(Object.values(o.terms.OnDemand[p.sku] ?? {})[0]?.priceDimensions ?? {})[0]?.pricePerUnit?.USD);
  const single = new Map(); // `${engine}/${name}` -> micro
  const multi = new Map();
  for (const p of Object.values(o.products)) {
    const a = p.attributes;
    const engine = a.licenseModel === "License included" ? EDITIONS[`${a.databaseEngine}|${a.databaseEdition}`] : undefined;
    // a region other than us-east-1 prefixes the usage type (USE2-InstanceUsage:...), and the usage type
    // abbreviates the size (db.m5.12xl): the instanceType attribute is the class's name
    const m = /^(?:[A-Z0-9]+-)?(InstanceUsage|Multi-AZUsage|MirrorUsage):db\./.exec(a.usagetype ?? "");
    if (!engine || !m || !(usd(p) > 0)) continue;
    const name = `aws/${a.instanceType}`.toLowerCase();
    if (!known.has(name)) {
      left.add(a.instanceType);
      continue;
    }
    (m[1] === "InstanceUsage" ? single : multi).set(`${engine}/${name}`, Math.round(usd(p) * 1e6));
  }
  for (const [k, v] of single) {
    const [engine, name] = [k.slice(0, k.indexOf("/")), k.slice(k.indexOf("/") + 1)];
    ((out[engine] ??= {})[region] ??= []).push([name, v]);
    const m = multi.get(k);
    if (m) {
      ratioMin = Math.min(ratioMin, m / v);
      ratioMax = Math.max(ratioMax, m / v);
      pairs++;
      if (Math.abs(m / v - 2) <= 0.01) nearTwo++;
    }
  }
  console.error(`${region}: ${[...single.keys()].length} prices`);
}
rmSync(tmp, { recursive: true, force: true });

const date = new Date().toISOString().slice(0, 10);
const providers = [];
for (const [engine, byRegion] of Object.entries(out)) {
  const provider = `aws-rds-${engine}`;
  providers.push(provider);
  const dir = new URL(`prices/${provider}/`, data);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  for (const [region, list] of Object.entries(byRegion)) {
    list.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
    writeFileSync(
      new URL(`${region}.csv`, dir),
      [
        `# On-demand price an hour in micro-USD (1000000 = 1 dollar) of ${provider}, licence included, Single-AZ, in region ${region}.`,
        `# Generated ${date} by scripts/gen-rds-licensed.mjs from AWS's AmazonRDS price list; do not edit by hand.`,
        "# name,usd_micro_hr. An estimate for a what-if, not a quote; a class missing here has no price there.",
        ...list.map((r) => r.join(",")),
      ].join("\n") + "\n",
    );
  }
}
const rs = new URL("../src/prices_data.rs", data);
const kept = readFileSync(rs, "utf8").split("\n").filter((l) => !providers.some((p) => l.includes(`("${p}",`)));
const end = kept.lastIndexOf("];");
const added = providers.flatMap((p) =>
  readdirSync(new URL(`prices/${p}/`, data))
    .map((f) => f.replace(/\.csv$/, ""))
    .sort()
    .map((r) => `    ("${p}", "${r}", include_str!("../data/prices/${p}/${r}.csv")),`),
);
kept.splice(end, 0, ...added);
writeFileSync(rs, kept.join("\n"));
console.error(`engines: ${providers.join(" ")}; ${added.length} region files`);
console.error(`classes not in the table, left out: ${[...left].sort().join(" ") || "none"}`);
console.error(`Multi-AZ price over Single-AZ, all classes read: ${ratioMin.toFixed(3)} to ${ratioMax.toFixed(3)}; ${nearTwo} of ${pairs} pairs are 2.00 (within 0.01)`);
