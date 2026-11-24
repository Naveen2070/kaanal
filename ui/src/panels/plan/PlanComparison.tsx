// The comparison across providers (step 9c-3), exactly as the record gives it: the providers not
// compared and why, then the rows in the server's order with its rank, then its conclusion verbatim.
// The UI ranks and picks nothing. A plan with no provider section (or from an older server) has no comparison: a line says so.

import type { Comparison } from "../../data/api/types.plan";
import { clean, price } from "./planMath";

const cell = "py-1 pr-2 align-top";
const head = "py-1 pr-2 font-normal";

export function PlanComparison({ comparison }: { comparison: Comparison | undefined }) {
  if (!comparison) {
    return <p className="text-xs text-muted">No comparison in this plan.</p>;
  }
  return (
    <div className="text-xs">
      {comparison.not_compared.length > 0 ? (
        <ul className="mb-2 space-y-0.5">
          {comparison.not_compared.map((n, i) => (
            <li key={i} className="[overflow-wrap:anywhere]">
              {`Not compared: ${clean(n.provider)}: ${clean(n.reason)}`}
            </li>
          ))}
        </ul>
      ) : null}
      <div className="overflow-x-auto">
        <table className="w-full border-collapse text-left">
          <caption className="pb-1 text-left text-sm font-medium">Comparison</caption>
          <thead>
            <tr className="text-muted">
              <th scope="col" className={head}>
                Rank
              </th>
              <th scope="col" className={head}>
                Design
              </th>
              <th scope="col" className={`${head} text-right`}>
                Price an hour
              </th>
              <th scope="col" className={head}>
                Nodes not priced
              </th>
              <th scope="col" className={head}>
                Holds
              </th>
              <th scope="col" className={head}>
                How it ended
              </th>
            </tr>
          </thead>
          <tbody>
            {comparison.rows.map((r, i) => (
              <tr key={i} className="border-t border-border">
                <td className={`${cell} tabular-nums`}>
                  {r.rank !== undefined ? r.rank : r.holds ? "not comparable" : "-"}
                </td>
                <td className={`${cell} max-w-48 [overflow-wrap:anywhere]`}>{clean(r.name)}</td>
                <td className={`${cell} text-right tabular-nums whitespace-nowrap`}>
                  {price(r.price_micro_usd_hr)}
                </td>
                <td className={`${cell} max-w-48 [overflow-wrap:anywhere]`}>
                  {r.unpriced === 0
                    ? "0"
                    : r.unpriced_nodes.length > 0
                      ? `${r.unpriced}: ${r.unpriced_nodes.map(clean).join(", ")}`
                      : String(r.unpriced)}
                </td>
                <td className={`${cell} ${r.holds ? "text-positive" : "text-negative"}`}>
                  {r.holds ? "yes" : "no"}
                </td>
                <td className={`${cell} [overflow-wrap:anywhere]`}>{clean(r.outcome)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="mt-2 [overflow-wrap:anywhere]">{clean(comparison.conclusion)}</p>
    </div>
  );
}
