// Plan records for tests: the shapes of the server's own golden reports
// (`crates/kaanal-server/tests/golden/plan-report*.md`, their Data blocks).

import type {
  PlanBranch,
  PlanDoc,
  PlanEffect,
  PlanMove,
  PlanRecord,
  PlanStep,
} from "../data/api/types.plan";

export const effect = (
  p99_ms: number,
  error_rate: number,
  capacity_req_s: number | null,
): PlanEffect => ({
  p99_ms,
  error_rate,
  capacity_req_s,
});

export const move = (node: string, kind: PlanMove["kind"]): PlanMove => ({ node, kind });

const step = (over: Partial<PlanStep> & Pick<PlanStep, "mv">): PlanStep => ({
  price_micro_usd_hr: null,
  price_delta_micro_usd_hr: null,
  before: effect(300, 0, 251),
  after: effect(520, 0.001, 255),
  menu: [],
  note: null,
  ...over,
});

const branch = (over: Partial<PlanBranch> = {}): PlanBranch => ({
  verdict: { kind: "holds" },
  steps: [],
  start: effect(300, 0, 251),
  end: effect(520, 0.001, 255),
  price_before: 1_620_000,
  price_after: 926_500,
  unpriced_nodes: [],
  ...over,
});

const request = {
  target: { multiple: 2.5 },
  factor: 2.5,
  req_s: 250,
  limit: { p99_ms: 1000, error_rate: 0.01 },
  budget_micro_usd_hr: 2_000_000,
  keep: ["cache"],
  max_steps: 30,
};

const sentence = "The plan holds the target.";
const budgetLine =
  "Budget check, with per-use prices counted at the target run's visits: the plan shown costs $1.6345/h, within the budget of $2.00/h.";

const verify = {
  holds: true,
  p99_ms: 300,
  error_rate: 0,
  capacity_req_s: 251,
  cost: { total_micro_usd_hr: 1_634_500, priced: 3, unpriced: 1 },
};

/** The two steps of the plain golden: db class up, then an app copy. */
export const plainSteps = (): PlanStep[] => [
  step({
    mv: move("db", { knob: "class", from: "db.m5.large", to: "db.m5.2xlarge" }),
    price_micro_usd_hr: 1_420_000,
    price_delta_micro_usd_hr: 580_000,
    before: effect(4100, 0.05, 49),
    after: effect(1200, 0.002, 118),
    menu: [
      {
        mv: move("db", { knob: "class", from: "db.m5.large", to: "db.m5.xlarge" }),
        price_micro_usd_hr: 1_130_000,
        effect: effect(2900, 0.015, 83),
        holds: false,
      },
      {
        mv: move("db", { knob: "copies", from: 1, to: 2 }),
        price_micro_usd_hr: 1_090_000,
        effect: effect(3300, 0.02, 77),
        holds: false,
      },
    ],
  }),
  step({
    mv: move("app", { knob: "copies", from: 2, to: 3 }),
    price_micro_usd_hr: 1_620_000,
    price_delta_micro_usd_hr: 200_000,
    before: effect(1200, 0.002, 118),
    after: effect(300, 0, 251),
    note: "a second copy of app halves its queue",
  }),
];

/** A finished plain plan that holds, with a family section and a proposed draft. */
export function planDone(): PlanRecord {
  const steps = plainSteps();
  const plan: PlanDoc = {
    ...branch({
      steps,
      start: effect(4100, 0.05, 49),
      end: effect(300, 0, 251),
      price_before: 840_000,
      price_after: 1_620_000,
      unpriced_nodes: ["cdn"],
    }),
    label:
      "relative what-if, not a prediction; prices are estimates in dollars an hour (fields named micro_usd are millionths of a dollar), of the priced nodes only",
    request,
    budget_check: { budget_micro_usd_hr: 2_000_000, cost_micro_usd_hr: 1_634_500, rejudged: false },
    families: [
      {
        kind: "answer",
        node: "db",
        flavor: "dynamodb",
        plan: branch({ steps: [], price_before: 700_000, price_after: 1_200_000 }),
        notes: ["assumes your application can use this family; preset estimates"],
      },
      {
        kind: "skipped",
        node: "db",
        flavor: "mongodb",
        reason: "no preset instance table for this flavor",
      },
    ],
    verify,
    measured_knee: { kind: "knee", k: 2.62, req_s: 262 },
    words: { verdict: sentence, budget: budgetLine, sections: [] },
  };
  return {
    id: "s4",
    kind: "plan",
    state: "done",
    generation: 2,
    profile: "peak",
    started_ms: 1,
    finished_ms: 2,
    plan_steps: steps,
    plan,
    proposed_draft: "version: 1\n",
    seed: 7,
    fingerprint: "0123456789ab",
  };
}

/** A cheapest plan: a search block, a comparison with one skipped provider, a provider section. */
export function planCheapest(): PlanRecord {
  const rec = planDone();
  const steps: PlanStep[] = [
    step({
      mv: move("app", { knob: "copies", from: 5, to: 3 }),
      price_delta_micro_usd_hr: -400_000,
      note: "cheaper by $0.40 an hour and still holds",
    }),
    step({
      mv: move("db", { knob: "class", from: "db.m5.2xlarge", to: "db.m5.xlarge" }),
      price_delta_micro_usd_hr: -290_000,
      note: "cheaper by $0.29 an hour and still holds",
    }),
    step({
      mv: move("worker", { knob: "class", from: "c5.xlarge", to: "c5.large" }),
      price_delta_micro_usd_hr: -3_500,
      note: "cheaper by $0.0035 an hour and still holds",
    }),
  ];
  const info = {
    stop: "run_cap" as const,
    runs: 200,
    run_cap: 200,
    elapsed_ms: 22_400,
    limit_ms: 37_500,
    design_in_force_holds: true,
    notes: [],
  };
  const gcp = branch({
    steps: [
      step({
        mv: move("app", { knob: "copies", from: 5, to: 3 }),
        price_delta_micro_usd_hr: -600_000,
        note: "cheaper by $0.60 an hour and still holds",
      }),
    ],
    price_before: 1_480_000,
    price_after: 880_000,
  });
  rec.plan_steps = steps;
  rec.plan = {
    ...rec.plan!,
    ...branch({ steps, price_before: 1_620_000, price_after: 926_500, unpriced_nodes: ["cdn"] }),
    families: [],
    verify: {
      ...verify,
      cost: { total_micro_usd_hr: 940_000, priced: 3, unpriced: 1 },
      p99_ms: 520,
      capacity_req_s: 255,
      error_rate: 0.001,
    },
    budget_check: { budget_micro_usd_hr: 2_000_000, cost_micro_usd_hr: 940_000, rejudged: false },
    request: {
      ...request,
      cheapest: { time_limit_s: 150, run_cap: 200, providers: ["gcp", "azure"] },
    },
    cheapest_note: "cheapest found within the limit, not proven cheapest",
    search: {
      stop: "exhausted",
      runs: 64,
      run_cap: 200,
      elapsed_ms: 31_800,
      limit_ms: 75_000,
      design_in_force_holds: true,
      notes: [],
    },
    providers: [
      {
        kind: "answer",
        provider: "gcp",
        plan: gcp,
        cost: { total_micro_usd_hr: 892_000, priced: 3, unpriced: 1 },
        budget_check: {
          budget_micro_usd_hr: 2_000_000,
          cost_micro_usd_hr: 892_000,
          rejudged: false,
        },
        info,
        notes: [
          "app: aws/m5.large to gcp/n2-standard-2",
          "nginx: kept as written, not compared (no class)",
        ],
      },
      {
        kind: "skipped",
        provider: "azure",
        reason: "db: no managed postgres class for azure in the price table",
      },
    ],
    comparison: {
      not_compared: [
        { provider: "Azure", reason: "db: no managed postgres class for azure in the price table" },
      ],
      rows: [
        {
          name: "the design translated to GCP",
          price_micro_usd_hr: 892_000,
          unpriced: 1,
          unpriced_nodes: [],
          holds: true,
          outcome: "holds",
          rank: 1,
        },
        {
          name: "the design in force, with the changes above",
          price_micro_usd_hr: 940_000,
          unpriced: 1,
          unpriced_nodes: [],
          holds: true,
          outcome: "holds",
          rank: 2,
        },
      ],
      comparable: true,
      conclusion:
        "Winner: the design translated to GCP, $0.892/h. It is the cheapest design that holds the target found within the limit, not proven cheapest.",
    },
    words: {
      verdict: sentence,
      budget:
        "Budget check, with per-use prices counted at the target run's visits: the plan shown costs $0.94/h, within the budget of $2.00/h.",
      sections: [sentence, ""],
    },
  };
  return rec;
}

/** A plan that could not hold: no proposed draft. */
export function planCannotHold(): PlanRecord {
  const rec = planDone();
  rec.plan = {
    ...rec.plan!,
    verdict: { kind: "cannot_hold", node: "db", reason: "its class is the largest the table has." },
    steps: [],
    families: [],
    words: {
      verdict:
        "The plan cannot hold the target by sizing at 'db': its class is the largest the table has. The design after the steps taken does not hold the target.",
      budget: null,
      sections: [],
    },
  };
  rec.plan_steps = [];
  rec.proposed_draft = null;
  return rec;
}

/** A plan still running: the first step is in, no answer yet. */
export function planRunning(): PlanRecord {
  const rec = planDone();
  return {
    ...rec,
    state: "running",
    finished_ms: null,
    plan: null,
    proposed_draft: null,
    plan_steps: (rec.plan_steps ?? []).slice(0, 1),
  };
}

export function planFailed(): PlanRecord {
  const rec = planRunning();
  return { ...rec, state: "failed", finished_ms: 2, error: "the plan failed: no profile" };
}
