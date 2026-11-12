#!/usr/bin/env node
// Reads Microsoft's own VM size pages and adds the network figures they state to scripts/azure-network.csv
// (RULES 129). The series pages are listed from Microsoft's public docs repository
// (MicrosoftDocs/azure-compute-docs, articles/virtual-machines/sizes) and read as rendered on
// learn.microsoft.com: each series page has a table whose columns are a size name and a network bandwidth
// in Mb/s. Nothing is guessed:
// - only a size already in data/instances.csv is added (the table is the list of sizes Kaanal knows);
// - only a plain whole number is taken: "16,000+" is a lower bound and a range is not a figure, so those
//   are left out, as in the rows already in the file;
// - a figure already in azure-network.csv is never changed; a page that disagrees with it is reported.
//
// Usage: node scripts/read-azure-network.mjs [--write]
// Without --write it prints what it would add. Afterwards run
//   node scripts/gen-instances.mjs --azure-net
// to put the figures into instances.csv.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const LEARN = "https://learn.microsoft.com/en-us/azure/virtual-machines/sizes/";
const API = "https://api.github.com/repos/MicrosoftDocs/azure-compute-docs/contents/articles/virtual-machines/sizes";
const curl = (u, extra = []) =>
  execFileSync("curl", ["-sS", "-L", "-m", "90", "-H", "User-Agent: kaanal-doc-reader", ...extra, u], { maxBuffer: 1 << 28 }).toString();

const csvUrl = new URL("./azure-network.csv", import.meta.url);
const known = new Map();
for (const l of readFileSync(csvUrl, "utf8").split(/\r?\n/)) {
  if (l && !l.startsWith("#")) known.set(l.split(",")[0], Number(l.split(",")[1]));
}
const ours = new Set();
for (const l of readFileSync(new URL("../crates/kaanal-core/data/instances.csv", import.meta.url), "utf8").split(/\r?\n/)) {
  if (l.startsWith("azure/")) ours.add(l.split(",")[0].slice("azure/".length));
}

const series = [];
for (const d of JSON.parse(curl(API)).filter((e) => e.type === "dir" && !/previous|retired|migrate|resize/.test(e.name))) {
  for (const f of JSON.parse(curl(`${API}/${d.name}`))) {
    if (f.type === "file" && /-series\.md$/.test(f.name)) series.push(`${d.name}/${f.name.replace(/\.md$/, "")}`);
  }
}
console.error(`${series.length} series pages`);

const text = (s) => s.replace(/<[^>]+>/g, " ").replace(/&nbsp;/g, " ").replace(/&amp;/g, "&").replace(/\s+/g, " ").trim();
const found = new Map();
const conflicts = [];
let failed = 0;
for (const path of series) {
  let html;
  try {
    html = curl(LEARN + path);
  } catch {
    failed++;
    continue;
  }
  for (const m of html.matchAll(/<table[\s\S]*?<\/table>/g)) {
    const rows = [...m[0].matchAll(/<tr[\s\S]*?<\/tr>/g)].map((r) => [...r[0].matchAll(/<t[hd][\s\S]*?<\/t[hd]>/g)].map((c) => text(c[0])));
    const head = rows[0] ?? [];
    const nameAt = head.findIndex((h) => /^size/i.test(h));
    const bwAt = head.findIndex((h) => /network bandwidth/i.test(h));
    if (nameAt < 0 || bwAt < 0) continue;
    for (const r of rows.slice(1)) {
      const name = (r[nameAt] ?? "").toLowerCase();
      const raw = (r[bwAt] ?? "").replace(/,/g, "");
      if (!/^standard_/.test(name) || !/^\d+$/.test(raw) || !ours.has(name)) continue;
      const mbps = Number(raw);
      if (known.has(name) && known.get(name) !== mbps) conflicts.push(`${name}: file ${known.get(name)}, ${path} says ${mbps}`);
      else if (!known.has(name)) found.set(name, mbps);
    }
  }
}
console.error(`${failed} pages failed; ${found.size} new sizes; ${conflicts.length} disagreements with the file`);
for (const c of conflicts) console.error(`  ${c}`);
const rows = [...found].sort(([a], [b]) => (a < b ? -1 : 1)).map(([n, v]) => `${n},${v}`);
if (process.argv.includes("--write")) {
  const old = readFileSync(csvUrl, "utf8");
  const nl = old.includes("\r\n") ? "\r\n" : "\n";
  writeFileSync(csvUrl, old.replace(/\s*$/, nl) + rows.join(nl) + nl);
  console.error(`wrote ${rows.length} rows to scripts/azure-network.csv`);
} else {
  console.log(rows.join("\n"));
}
