#!/usr/bin/env node
// Azure SQL Database vCore classes (RULES 131, B13): General Purpose and Business Critical, provisioned,
// Gen5 hardware, from Azure's public retail prices API (prices.azure.com, no key). Writes
//   - crates/kaanal-core/data/prices/azure-sql/<region>.csv   (name,usd_micro_hr per region)
//   - the `azure/db.gp_gen5_<n>` and `azure/db.bc_gen5_<n>` rows of data/instances.csv
//   - the `azure-sql` lines of src/prices_data.rs
// and keeps every other row, folder and line as it is. Run: node scripts/gen-azure-sql.mjs
//
// What is read, and what is not (an estimate for a what-if, never a quote):
// - price of a class = n vCores x the per-vCore rate of the tier in the region. The rate is the median of
//   price / n over the API's "<n> vCore" rows of the tier's Compute Gen5 product (they agree to the sixth
//   decimal). That is the compute meter: storage, backup storage, the zone-redundant add-on and any SQL
//   licence meter are not in it (the API lists no licence meter for SQL Database).
// - vCPU = n; memory = n x 5.1875 GiB: Microsoft's resource-limit page lists 10.4, 20.8, 31.1, 41.5, 51.9,
//   62.3, 72.7, 83, 93.4 and 103.8 GB for 2 to 20 vCores, which is 5.1875 GB a vCore, applied to the sizes
//   it does not list (24, 32, 40, 80).
// - net_mbps 0: Microsoft states no figure.
// - the API names a region `eastus`; the VM price files use `us-east` (scripts/azure-regions.mjs joins them);
//   a region with no VM name is reported and left out.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { vmName } from "./azure-regions.mjs";

const data = new URL("../crates/kaanal-core/data/", import.meta.url);
const SIZES = [2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 24, 32, 40, 80];
const GIB_PER_VCORE_MIB = 5312; // 5.1875 GiB
const TIERS = {
  gp: "SQL Database Single/Elastic Pool General Purpose - Compute Gen5",
  bc: "SQL Database Single/Elastic Pool Business Critical - Compute Gen5",
};
const PROVIDER = "azure-sql";
const DEFAULT_REGION = "us-east"; // the table's price column, like the VM rows

const filter =
  `serviceName eq 'SQL Database' and priceType eq 'Consumption' and meterName eq 'vCore' and (productName eq '${TIERS.gp}' or productName eq '${TIERS.bc}')`;
let url = "https://prices.azure.com/api/retail/prices?api-version=2023-01-01-preview&$filter=" + encodeURIComponent(filter);
const perVcore = {}; // arm region -> tier -> [rate, ...]
while (url) {
  const j = JSON.parse(execFileSync("curl", ["-sS", "-L", "-m", "120", url], { maxBuffer: 1 << 28 }).toString());
  for (const i of j.Items) {
    const m = /^(\d+) vCore$/.exec(i.skuName);
    const tier = Object.keys(TIERS).find((t) => TIERS[t] === i.productName);
    if (!m || !tier || !(i.retailPrice > 0)) continue;
    (((perVcore[i.armRegionName] ??= {})[tier]) ??= []).push(i.retailPrice / Number(m[1]));
  }
  url = j.NextPageLink;
}

const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
};

const date = new Date().toISOString().slice(0, 10);
const dir = new URL(`prices/${PROVIDER}/`, data);
rmSync(dir, { recursive: true, force: true });
mkdirSync(dir, { recursive: true });
const unmatched = [];
const regions = [];
let defaultPrices = null;
for (const [arm, tiers] of Object.entries(perVcore).sort()) {
  if (!tiers.gp || !tiers.bc) continue;
  const region = vmName(arm);
  if (!region) {
    unmatched.push(arm);
    continue;
  }
  const rate = { gp: median(tiers.gp), bc: median(tiers.bc) };
  const list = [];
  for (const t of ["bc", "gp"]) for (const n of SIZES) list.push([`azure/db.${t}_gen5_${n}`, Math.round(rate[t] * n * 1e6)]);
  list.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  writeFileSync(
    new URL(`${region}.csv`, dir),
    [
      `# On-demand compute price an hour in micro-USD (1000000 = 1 dollar) of ${PROVIDER} in region ${region}: vCores x the per-vCore rate.`,
      `# Generated ${date} by scripts/gen-azure-sql.mjs from Azure's retail prices API; do not edit by hand.`,
      "# name,usd_micro_hr. Compute only (no storage, backup or licence meter). An estimate for a what-if, not a quote.",
      ...list.map((r) => r.join(",")),
    ].join("\n") + "\n",
  );
  regions.push(region);
  if (region === DEFAULT_REGION) defaultPrices = new Map(list);
}
if (!defaultPrices) throw new Error(`no ${DEFAULT_REGION} price: the table's price column needs it`);

// instances.csv: replace the azure/db.* rows
const csvUrl = new URL("instances.csv", data);
const lines = readFileSync(csvUrl, "utf8").split(/\r?\n/).filter((l) => l && !l.startsWith("azure/db."));
const head = lines.filter((l) => l.startsWith("#"));
const rows = lines.filter((l) => !l.startsWith("#"));
for (const t of ["bc", "gp"]) {
  for (const n of SIZES) {
    const name = `azure/db.${t}_gen5_${n}`;
    rows.push([name, n, n * GIB_PER_VCORE_MIB, 0, defaultPrices.get(name)].join(","));
  }
}
rows.sort((a, b) => {
  const x = a.split(",")[0];
  const y = b.split(",")[0];
  return x < y ? -1 : x > y ? 1 : 0;
});
writeFileSync(csvUrl, head.concat(rows).join("\n") + "\n");

// prices_data.rs: drop the old azure-sql lines, add the new ones at the end of the table
const rs = new URL("../src/prices_data.rs", data);
const kept = readFileSync(rs, "utf8").split("\n").filter((l) => !l.includes(`("${PROVIDER}",`));
const end = kept.lastIndexOf("];");
kept.splice(end, 0, ...regions.sort().map((r) => `    ("${PROVIDER}", "${r}", include_str!("../data/prices/${PROVIDER}/${r}.csv")),`));
writeFileSync(rs, kept.join("\n"));
console.error(`azure-sql: ${regions.length} regions, ${SIZES.length * 2} classes; no VM-region name (left out): ${unmatched.join(" ") || "none"}`);
