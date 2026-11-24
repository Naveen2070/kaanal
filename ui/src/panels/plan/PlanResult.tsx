// What a finished plan says (step 9c-3), all from the record: the verdict and budget sentences the
// server words (`words`), the changes, the comparison and provider sentences, the database families,
// the checked line and how the cheapest search ended. An older server's record has no `words`: the
// verdict is then one plain line from its kind, and nothing else is worded here.

import type { PlanDoc } from "../../data/api/types.plan";
import { PlanComparison } from "./PlanComparison";
import { PlanFamilies } from "./PlanFamilies";
import { PlanSearch } from "./PlanSearch";
import { PlanSteps } from "./PlanSteps";
import { clean, effectWords, n1 } from "./planMath";

/** The re-check and the sweep's knee, as the report's "Checked" section words them. */
function checked(p: PlanDoc): string[] {
  const v = p.verify;
  const lines = [
    `Checked again at the target: ${effectWords(v)}. It ${v.holds ? "holds" : "does not hold"} the target with the margin.`,
  ];
  const k = p.measured_knee;
  if (k) {
    const at = `${n1(k.req_s)} req/s (${k.k.toFixed(2)}x the profile's rates)`;
    lines.push(
      k.kind === "knee"
        ? `A load sweep of the proposed design finds its knee near ${at}.`
        : k.kind === "broken_at_lowest"
          ? `A load sweep of the proposed design finds it over the limit already at its lowest point, ${at}.`
          : `A load sweep of the proposed design finds no point over the limit up to ${at}, the highest it ran.`,
    );
  } else if (p.sweep_error) {
    lines.push(
      `The load sweep of the proposed design did not run: ${clean(p.sweep_error).replace(/\.$/, "")}.`,
    );
  }
  return lines;
}

export function PlanResult({ plan }: { plan: PlanDoc }) {
  const w = plan.words;
  const sections = (plan.providers ?? []).flatMap((s, i) => {
    const said = w?.sections[i];
    return s.kind === "answer" && said ? [`${clean(s.provider)}: ${clean(said)}`] : [];
  });
  const cheapest = plan.request.cheapest;
  return (
    <div className="space-y-3 text-xs">
      <div className="rounded-lg border border-border p-2">
        <p className="m-0 text-sm font-medium [overflow-wrap:anywhere]">
          {w
            ? clean(w.verdict)
            : `Verdict: ${plan.verdict.kind.replace(/_/g, " ")} (this server sent no sentence for it; the report has it).`}
        </p>
        {w?.budget ? <p className="m-0 mt-1 [overflow-wrap:anywhere]">{clean(w.budget)}</p> : null}
      </div>
      {plan.steps.length > 0 ? <PlanSteps steps={plan.steps} /> : <p>No changes.</p>}
      {plan.comparison ? <PlanComparison comparison={plan.comparison} /> : null}
      {sections.length > 0 ? (
        <ul className="space-y-0.5">
          {sections.map((s, i) => (
            <li key={i} className="[overflow-wrap:anywhere]">
              {s}
            </li>
          ))}
        </ul>
      ) : null}
      <PlanFamilies families={plan.families} />
      <div>
        {checked(plan).map((l, i) => (
          <p key={i} className="m-0 [overflow-wrap:anywhere]">
            {l}
          </p>
        ))}
      </div>
      <PlanSearch
        search={plan.search}
        cheapestNote={plan.cheapest_note}
        share={
          (plan.providers?.length ?? 0) > 0 && cheapest
            ? { kind: "search", timeLimitS: cheapest.time_limit_s }
            : undefined
        }
      />
    </div>
  );
}
