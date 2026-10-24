#!/usr/bin/env node
// Benchmark helpers for scripts/bench.sh and the bench workflows. Node standard library only.
//
//   merge   RUN.json...                           merged run on stdout: median of the medians,
//                                                 lowest min, highest max
//   compare BASE.json NEW.json                    markdown table: % change of every median
//   append  HISTORY.json RUN.json [key=value...]  adds one entry (created if missing); the keys
//                                                 are free text: commit, ref, os, note, runs
//   report  HISTORY.json                          markdown cumulative report over the history
//
// A run file is what `cargo test -p kaanal --release --test bench -- --ignored` writes:
// { "reps": 5, "rows": [{ "name", "unit", "median", "min", "max" }, ...] }.
// Shared CI runners are noisy (2x swings between runs are normal), so a change is only called
// out beyond +-25%; read the numbers as trends over several runs, not as a single verdict.

import fs from "node:fs";

const NOISE = 25; // percent
const LAST = 8; // runs shown in the cumulative report

// Files can come from artifacts of earlier runs, so check their shape and escape every string before
// it lands in markdown (a job summary renders it).
const clean = (s) => String(s).replace(/[|<>`\r\n]/g, " ").slice(0, 120);
const isName = (s) => typeof s === "string" && /^[A-Za-z0-9_./-]{1,64}$/.test(s);
const isUnit = (s) => typeof s === "string" && /^[a-z/]{1,10}$/.test(s);
const isNum = (x) => typeof x === "number" && Number.isFinite(x);
const checkRow = (r) => {
  if (!r || !isNum(r.median) || !isNum(r.min) || !isNum(r.max) || !isUnit(r.unit)) throw new Error("bad bench row");
};
const checkRun = (run) => {
  if (!run || !Array.isArray(run.rows) || run.rows.length > 200) throw new Error("bad run file");
  for (const r of run.rows) {
    if (!isName(r.name)) throw new Error("bad bench row name");
    checkRow(r);
  }
  return run;
};
const checkHistory = (h) => {
  if (!Array.isArray(h) || h.length > 1000) throw new Error("bad history file");
  for (const e of h) {
    if (!e || typeof e !== "object" || typeof e.date !== "string" || !e.rows || typeof e.rows !== "object") throw new Error("bad history entry");
    for (const [name, r] of Object.entries(e.rows)) {
      if (!isName(name)) throw new Error("bad history row name");
      checkRow(r);
    }
  }
  return h;
};
const read = (p) => JSON.parse(fs.readFileSync(p, "utf8"));
const readRun = (p) => checkRun(read(p));
const readHistory = (p) => checkHistory(read(p));
const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};
// req/s is better when higher; every other unit (ns/op, us, ms) when lower.
const higherIsBetter = (unit) => unit === "req/s";
const fmt = (x) => (x >= 100 ? x.toFixed(0) : x >= 10 ? x.toFixed(1) : x.toFixed(2));
const pct = (base, now) => ((now - base) / base) * 100;
const sign = (p) => `${p >= 0 ? "+" : ""}${p.toFixed(1)}%`;
const verdict = (p, unit) => {
  if (Math.abs(p) <= NOISE) return "~";
  return (p > 0) === higherIsBetter(unit) ? "better" : "worse";
};
const rowsByName = (run) => new Map(run.rows.map((r) => [r.name, r]));

function merge(files) {
  const runs = files.map(readRun);
  const names = runs[0].rows.map((r) => r.name);
  const rows = names.map((name) => {
    const rs = runs.map((r) => rowsByName(r).get(name)).filter(Boolean);
    return {
      name,
      unit: rs[0].unit,
      median: median(rs.map((r) => r.median)),
      min: Math.min(...rs.map((r) => r.min)),
      max: Math.max(...rs.map((r) => r.max)),
    };
  });
  const reps = runs.reduce((n, r) => n + (r.reps || 0), 0);
  process.stdout.write(JSON.stringify({ reps, runs: runs.length, rows }, null, 2) + "\n");
}

function compare(basePath, newPath) {
  const base = rowsByName(readRun(basePath));
  const out = ["| bench | baseline | now | change | unit | |", "| --- | ---: | ---: | ---: | --- | --- |"];
  for (const r of readRun(newPath).rows) {
    const b = base.get(r.name);
    if (!b) {
      out.push(`| ${r.name} | n/a | ${fmt(r.median)} | new | ${r.unit} | |`);
      continue;
    }
    const p = pct(b.median, r.median);
    out.push(`| ${r.name} | ${fmt(b.median)} | ${fmt(r.median)} | ${sign(p)} | ${r.unit} | ${verdict(p, r.unit)} |`);
  }
  out.push("", `Medians of the suite's repetitions. Changes within +-${NOISE}% are noise on a shared runner; the only hard check is NFR-05 (1M simulated requests under 10 s).`);
  process.stdout.write(out.join("\n") + "\n");
}

function append(historyPath, runPath, pairs) {
  const history = fs.existsSync(historyPath) ? readHistory(historyPath) : [];
  const entry = { date: new Date().toISOString(), ...Object.fromEntries(pairs.map((kv) => {
    const i = kv.indexOf("=");
    return [kv.slice(0, i), kv.slice(i + 1)];
  })) };
  entry.rows = Object.fromEntries(readRun(runPath).rows.map((r) => [r.name, { unit: r.unit, median: r.median, min: r.min, max: r.max }]));
  history.push(entry);
  fs.writeFileSync(historyPath, JSON.stringify(history, null, 2) + "\n");
  console.log(`history now holds ${history.length} run(s)`);
}

function report(historyPath) {
  const history = readHistory(historyPath);
  if (!history.length) {
    console.log("No runs recorded yet.");
    return;
  }
  const shown = history.slice(-LAST);
  const label = (e) => `${clean(e.commit || "?").slice(0, 7)} ${clean(e.date).slice(0, 10)}`;
  const out = [`# Benchmark history (${history.length} run${history.length === 1 ? "" : "s"}, last ${shown.length} shown)`, ""];
  out.push("| # | date | commit | ref | os | runs | note |", "| ---: | --- | --- | --- | --- | ---: | --- |");
  shown.forEach((e, i) => out.push(`| ${history.length - shown.length + i + 1} | ${clean(e.date).slice(0, 16).replace("T", " ")} | ${clean(e.commit || "?").slice(0, 7)} | ${clean(e.ref || "")} | ${clean(e.os || "")} | ${clean(e.runs || "")} | ${clean(e.note || "")} |`));
  out.push("", "Each cell is the median of the run's medians, in the row's unit. `vs first` and `vs last` compare the newest run with the oldest and the previous one shown.", "");
  const names = Object.keys(shown[shown.length - 1].rows);
  out.push(`| bench | unit | ${shown.map(label).join(" | ")} | vs first | vs last |`);
  out.push(`| --- | --- | ${shown.map(() => "---:").join(" | ")} | ---: | ---: |`);
  for (const name of names) {
    const cells = shown.map((e) => e.rows[name]);
    const unit = (cells.find(Boolean) || {}).unit || "";
    const now = cells[cells.length - 1];
    const first = cells.find(Boolean);
    const prev = cells.length > 1 ? cells[cells.length - 2] : null;
    const rel = (b) => (b && now && b !== now ? `${sign(pct(b.median, now.median))} ${verdict(pct(b.median, now.median), unit)}` : "");
    out.push(`| ${name} | ${unit} | ${cells.map((c) => (c ? fmt(c.median) : "")).join(" | ")} | ${rel(first)} | ${rel(prev)} |`);
  }
  out.push("", `Changes within +-${NOISE}% are marked \`~\`: shared CI runners are noisy, so judge trends over several runs.`);
  process.stdout.write(out.join("\n") + "\n");
}

const [cmd, ...args] = process.argv.slice(2);
const usage = () => {
  console.error("usage: bench-report.mjs merge RUN... | compare BASE NEW | append HISTORY RUN [k=v...] | report HISTORY");
  process.exit(64);
};
if (cmd === "merge" && args.length) merge(args);
else if (cmd === "compare" && args.length === 2) compare(args[0], args[1]);
else if (cmd === "append" && args.length >= 2) append(args[0], args[1], args.slice(2));
else if (cmd === "report" && args.length === 1) report(args[0]);
else usage();
