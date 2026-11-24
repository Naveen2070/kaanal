// The database-family section of a plan (step 9c-3): one collapsed <details> per answer or skipped
// entry, with the server's notes or reason as given.

import type { FamilySection } from "../../data/api/types.plan";
import { clean, price } from "./planMath";

export function PlanFamilies({ families }: { families: FamilySection[] }) {
  if (families.length === 0) return null;
  return (
    <div className="space-y-1 text-xs">
      {families.map((f, i) => (
        <details key={i} className="rounded border border-border px-2 py-1">
          <summary className="cursor-pointer [overflow-wrap:anywhere]">
            {f.kind === "answer"
              ? `${clean(f.node)} as ${clean(f.flavor)}`
              : `${clean(f.node)}${f.flavor ? ` as ${clean(f.flavor)}` : ""}: not walked`}
          </summary>
          {f.kind === "answer" ? (
            <>
              <p className="mt-1">
                {`${f.plan.steps.length} ${f.plan.steps.length === 1 ? "change" : "changes"}, price ${price(f.plan.price_before)} to ${price(f.plan.price_after)}`}
              </p>
              {f.notes.length > 0 ? (
                <ul className="mt-1 space-y-0.5 text-muted">
                  {f.notes.map((n, j) => (
                    <li key={j} className="[overflow-wrap:anywhere]">
                      {clean(n)}
                    </li>
                  ))}
                </ul>
              ) : null}
            </>
          ) : (
            <p className="mt-1 text-muted [overflow-wrap:anywhere]">{clean(f.reason)}</p>
          )}
        </details>
      ))}
    </div>
  );
}
