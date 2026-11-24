// The wire types of a sizing plan (steps 9c-1 and 9c-2): the body of `POST /sim/plan` and the record
// `GET /sim/runs/{id}` gives for `kind: "plan"`. Money is micro-USD an hour (1,000,000 = one dollar),
// an estimate. `words` carries the report's own sentences, so a screen never words a verdict itself.

import type { SweepVerdict } from "./types";

/** The body of `POST /sim/plan`. `error_pct` is a percentage; `budget_per_hour` is dollars an hour. */
export interface PlanBody {
  target: { multiple: number } | { req_s: number };
  profile?: string;
  seed?: number;
  scenario?: string;
  p99_ms?: number;
  error_pct?: number;
  budget_per_hour?: number;
  keep?: string[];
  max_steps?: number;
  /** Also look for the cheapest design (step 9c-2). */
  cheapest?: boolean;
  /** Seconds, 1 to 600; needs `cheapest`. */
  time_limit_s?: number;
  /** Runs, 1 to 2000; needs `cheapest`. */
  run_cap?: number;
  /** `aws`, `gcp`, `azure`, or `["none"]`; needs `cheapest`. */
  providers?: string[];
}

/** One run's figures at the target. */
export interface PlanEffect {
  p99_ms: number;
  error_rate: number;
  capacity_req_s: number | null;
}

/** One size knob of one node, moved. */
export type PlanMoveKind =
  | { knob: "class"; from: string; to: string }
  | { knob: "cores"; from: number; to: number }
  | { knob: "memory"; from: number; to: number }
  | { knob: "copies"; from: number; to: number };

export interface PlanMove {
  node: string;
  kind: PlanMoveKind;
}

/** A move the walk tried at a step and did not necessarily take. */
export interface PlanCandidate {
  mv: PlanMove;
  price_micro_usd_hr: number | null;
  effect: PlanEffect;
  holds: boolean;
}

export interface PlanStep {
  mv: PlanMove;
  /** The whole design's price after the step; null when merged (cheapest) or nothing is priced. */
  price_micro_usd_hr: number | null;
  /** What this step adds (negative: takes off). */
  price_delta_micro_usd_hr: number | null;
  before: PlanEffect;
  after: PlanEffect;
  menu: PlanCandidate[];
  note: string | null;
}

export type PlanVerdict =
  | { kind: "already_holds" }
  | { kind: "holds" }
  | { kind: "cannot_hold"; node: string | null; reason: string }
  | { kind: "over_budget"; over_by_micro_usd_hr: number; holds: boolean }
  | { kind: "no_progress"; node: string }
  | { kind: "step_cap" };

export interface PlanCost {
  total_micro_usd_hr: number;
  priced: number;
  unpriced: number;
}

export interface PlanVerify {
  holds: boolean;
  p99_ms: number;
  error_rate: number;
  capacity_req_s: number | null;
  cost: PlanCost;
  cost_note?: string;
}

export interface BudgetCheck {
  budget_micro_usd_hr: number;
  cost_micro_usd_hr: number;
  rejudged: boolean;
}

/** How a search ended. */
export interface SearchInfo {
  stop: "exhausted" | "run_cap" | "time_limit";
  runs: number;
  run_cap: number;
  elapsed_ms: number;
  limit_ms: number;
  design_in_force_holds: boolean;
  notes: string[];
}

/** The plan of a design (the main path, a family alternative or a provider section). */
export interface PlanBranch {
  verdict: PlanVerdict;
  steps: PlanStep[];
  start: PlanEffect;
  end: PlanEffect;
  price_before: number | null;
  price_after: number | null;
  unpriced_nodes: string[];
}

export type FamilySection =
  | { kind: "answer"; node: string; flavor: string; plan: PlanBranch; notes: string[] }
  | { kind: "skipped"; node: string; flavor?: string | null; reason: string };

export type ProviderSection =
  | {
      kind: "answer";
      provider: string;
      plan: PlanBranch;
      cost: PlanCost;
      cost_note?: string;
      budget_check?: BudgetCheck;
      info: SearchInfo;
      notes: string[];
    }
  | { kind: "skipped"; provider: string; reason: string };

export interface CompareRow {
  name: string;
  price_micro_usd_hr: number;
  unpriced: number;
  unpriced_nodes: string[];
  holds: boolean;
  over_budget_micro_usd_hr?: number;
  outcome: string;
  /** Place among the holders; absent for one that does not hold or when they cannot be ranked. */
  rank?: number;
}

export interface Comparison {
  not_compared: { provider: string; reason: string }[];
  rows: CompareRow[];
  comparable: boolean;
  /** The winner, or why there is none. */
  conclusion: string;
}

/** The report's sentences as plain text. `sections` lines up with `providers` (`""` when skipped). */
export interface PlanWords {
  verdict: string;
  budget: string | null;
  sections: string[];
}

/** A finished plan. Fields an older server does not send are optional. */
export interface PlanDoc extends PlanBranch {
  label: string;
  request: {
    target: { multiple?: number; req_s?: number };
    factor: number;
    req_s: number;
    limit: { p99_ms: number; error_rate: number };
    budget_micro_usd_hr: number | null;
    keep: string[];
    max_steps: number;
    cheapest?: { time_limit_s: number; run_cap: number; providers: string[] };
  };
  budget_check?: BudgetCheck;
  families: FamilySection[];
  verify: PlanVerify;
  measured_knee: SweepVerdict | null;
  sweep_error?: string;
  cheapest_note?: string;
  search?: SearchInfo;
  providers?: ProviderSection[];
  comparison?: Comparison;
  words?: PlanWords;
}

/** A plan's record from `GET /sim/runs/{id}`. */
export interface PlanRecord {
  id: string;
  kind: "plan";
  state: "running" | "done" | "failed";
  generation: number;
  profile: string;
  started_ms: number;
  finished_ms: number | null;
  /** The steps as the walk takes them; kept once it is done. Absent (not empty) before the first step. */
  plan_steps?: PlanStep[];
  plan?: PlanDoc | null;
  /** The design with the plan applied, as text; absent when no plan was found. */
  proposed_draft?: string | null;
  seed?: number;
  fingerprint?: string;
  error?: string | null;
}
