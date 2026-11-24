// What the plan dialog does with numbers and words that the server does not already give as
// sentences: the form's rules (the same bounds the server checks, for a clear message before a
// request), money as the report prints it, a move in words, and whether a plan is stale. Verdicts and
// the comparison's conclusion are never worded here: the record carries them (`words`, `comparison`).

import type { PlanBody, PlanEffect, PlanMove, PlanRecord } from "../../data/api/types.plan";

export type Provider = "aws" | "gcp" | "azure";

export interface FormState {
  targetKind: "multiple" | "req_s";
  target: string;
  p99: string;
  err: string;
  budget: string;
  keep: string[];
  cheapest: boolean;
  time: string;
  runCap: string;
  /** The providers to compare, or `"none"`; `[]` lets the server choose. */
  providers: Provider[] | "none";
}

export const DEFAULT_FORM: FormState = {
  targetKind: "multiple",
  target: "2",
  p99: "1000",
  err: "1",
  budget: "",
  keep: [],
  cheapest: false,
  time: "150",
  runCap: "200",
  providers: [],
};

/** Highest p99 limit taken, ms (about 11.6 days): past it the number is a typo, not a limit. */
const MAX_P99_MS = 1e9;

const num = (s: string): number => (s.trim() === "" ? Number.NaN : Number(s));

export type Problems = Partial<
  Record<"target" | "p99" | "err" | "budget" | "time" | "runCap", string>
>;

/** The request the form asks for, or why a field cannot be sent. Bounds are the server's. */
export function toBody(f: FormState): { body: PlanBody } | { problems: Problems } {
  const problems: Problems = {};
  const target = num(f.target);
  if (f.targetKind === "multiple") {
    if (!(target >= 0.1 && target <= 64)) {
      problems.target = "The target must be 0.1 to 64 times the profile's rates.";
    }
  } else if (!(target > 0 && Number.isFinite(target))) {
    problems.target = "The target must be above 0 req/s.";
  }
  if (f.p99.trim() === "") problems.p99 = "Type a p99 limit in ms.";
  else {
    const n = Number(f.p99);
    if (!(Number.isFinite(n) && n > 0 && n <= MAX_P99_MS)) {
      problems.p99 = "The p99 limit must be above 0 ms and at most 1,000,000,000 ms.";
    }
  }
  if (f.err.trim() === "") problems.err = "Type an error limit in %.";
  else {
    const n = Number(f.err);
    if (!(Number.isFinite(n) && n >= 0 && n <= 100))
      problems.err = "The error limit must be 0 to 100 %.";
  }
  const budget = num(f.budget);
  if (f.budget.trim() !== "" && !(budget > 0 && Number.isFinite(budget))) {
    problems.budget = "The budget must be above 0 dollars an hour.";
  }
  const time = Math.ceil(num(f.time));
  const runCap = num(f.runCap);
  if (f.cheapest) {
    if (!(time >= 1 && time <= 600)) problems.time = "The time limit must be 1 to 600 seconds.";
    if (!(Number.isInteger(runCap) && runCap >= 1 && runCap <= 2000)) {
      problems.runCap = "The run cap must be a whole number from 1 to 2000.";
    }
  }
  if (Object.keys(problems).length > 0) return { problems };
  const body: PlanBody = {
    target: f.targetKind === "multiple" ? { multiple: target } : { req_s: target },
    p99_ms: Number(f.p99),
    error_pct: Number(f.err),
  };
  if (f.budget.trim() !== "") body.budget_per_hour = budget;
  if (f.keep.length > 0) body.keep = f.keep;
  if (f.cheapest) {
    body.cheapest = true;
    body.time_limit_s = time;
    body.run_cap = runCap;
    if (f.providers === "none") body.providers = ["none"];
    else if (f.providers.length > 0) body.providers = f.providers;
  }
  return { body };
}

/** Dollars an hour from micro-USD, exact: at least two decimals, never `0.00` for a real saving. */
export function dollars(micro: number): string {
  const whole = Math.trunc(micro / 1_000_000);
  let frac = String(micro % 1_000_000).padStart(6, "0");
  while (frac.length > 2 && frac.endsWith("0")) frac = frac.slice(0, -1);
  return `${whole}.${frac}`;
}

/** An hourly price, `$0.84/h`, or `not priced`. */
export const price = (micro: number | null): string =>
  micro === null ? "not priced" : `$${dollars(micro)}/h`;

/** What a step adds or takes off: `+$0.096/h`, `-$0.40/h`, or `not priced`. */
export function priceEffect(delta: number | null): string {
  if (delta === null) return "not priced";
  if (delta === 0) return "no price change";
  return `${delta < 0 ? "-" : "+"}$${dollars(Math.abs(delta))}/h`;
}

const MIB = 1 << 20;
const GIB = 2 ** 30;
const bytes = (b: number): string =>
  b >= GIB && b % GIB === 0
    ? `${b / GIB} GiB`
    : b >= MIB && b % MIB === 0
      ? `${b / MIB} MiB`
      : `${b} bytes`;

/** A name from the server with control and format characters taken out (a bidi override can reorder text). */
export const clean = (s: string): string => s.replace(/[\p{Cc}\p{Cf}]/gu, "");

/** A move in words, as the report writes it. */
export function changeWords(mv: PlanMove): string {
  const k = mv.kind;
  switch (k.knob) {
    case "class":
      return `instance class ${clean(k.from)} to ${clean(k.to)}`;
    case "cores":
      return `cores ${k.from} to ${k.to}`;
    case "memory":
      return `memory limit ${bytes(k.from)} to ${bytes(k.to)}`;
    case "copies":
      return `copies ${k.from} to ${k.to}`;
  }
}

/** A figure with one decimal and no trailing zero: `250`, `118.4`. */
export const n1 = (x: number): string => x.toFixed(1).replace(/\.?0+$/, "");

/** An error share as a percentage, two decimals, `<0.01%` for a share too small for them. */
export function errorWords(share: number): string {
  const p = share * 100;
  if (p > 0 && p < 0.005) return "<0.01%";
  return `${p.toFixed(2).replace(/\.?0+$/, "")}%`;
}

/** One run's figures in words. */
export const effectWords = (e: PlanEffect): string =>
  `p99 ${n1(e.p99_ms)} ms, errors ${errorWords(e.error_rate)}, capacity est. ${
    e.capacity_req_s === null ? "no estimate" : `${n1(e.capacity_req_s)} req/s`
  }`;

/** Whether the plan was made on another design than the one in force: after a server restart, or an Apply. */
export function isStale(
  rec: Pick<PlanRecord, "generation">,
  view: { generation: number; boot: string },
  sentBoot: string | null,
): "restarted" | "earlier" | null {
  if (sentBoot !== null && sentBoot !== view.boot) return "restarted";
  return rec.generation !== view.generation ? "earlier" : null;
}
