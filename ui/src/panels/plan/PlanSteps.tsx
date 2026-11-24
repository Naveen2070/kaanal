// The changes of a plan (step 9c-3): one table row per step as the record gives it, with the step's
// menu (the moves the walk tried there) in a collapsed, read-only <details> below its row. Names from
// the server go through clean(); React escapes the rest, so a name never runs as markup.

import { Fragment } from "react";
import type { PlanEffect, PlanStep } from "../../data/api/types.plan";
import { changeWords, clean, effectWords, errorWords, n1, price, priceEffect } from "./planMath";

const cell = "py-1 pr-2 align-top";
const head = "py-1 pr-2 font-normal";

const figures = (a: PlanEffect, b: PlanEffect): string =>
  `p99 ${n1(a.p99_ms)} to ${n1(b.p99_ms)} ms, errors ${errorWords(a.error_rate)} to ${errorWords(b.error_rate)}`;

export function PlanSteps({ steps }: { steps: PlanStep[] }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full border-collapse text-left text-xs">
        <caption className="pb-1 text-left text-sm font-medium">Changes</caption>
        <thead>
          <tr className="text-muted">
            <th scope="col" className={head}>
              #
            </th>
            <th scope="col" className={head}>
              Node
            </th>
            <th scope="col" className={head}>
              Change
            </th>
            <th scope="col" className={`${head} text-right`}>
              Price effect
            </th>
            <th scope="col" className={head}>
              p99 and errors, before to after
            </th>
            <th scope="col" className={head}>
              Note
            </th>
          </tr>
        </thead>
        <tbody>
          {steps.map((s, i) => (
            <Fragment key={i}>
              <tr className="border-t border-border">
                <td className={`${cell} tabular-nums`}>{i + 1}</td>
                <td className={`${cell} max-w-40 font-medium [overflow-wrap:anywhere]`}>
                  {clean(s.mv.node)}
                </td>
                <td className={`${cell} [overflow-wrap:anywhere]`}>{changeWords(s.mv)}</td>
                <td
                  className={`${cell} text-right tabular-nums whitespace-nowrap ${
                    s.price_delta_micro_usd_hr !== null && s.price_delta_micro_usd_hr < 0
                      ? "text-positive"
                      : ""
                  }`}
                >
                  {priceEffect(s.price_delta_micro_usd_hr)}
                </td>
                <td className={`${cell} tabular-nums`}>{figures(s.before, s.after)}</td>
                <td className={`${cell} text-muted [overflow-wrap:anywhere]`}>
                  {s.note === null ? "-" : clean(s.note)}
                </td>
              </tr>
              {s.menu.length > 0 ? (
                <tr>
                  <td
                    colSpan={6}
                    className="pb-1"
                    aria-label={`Other moves tried at step ${i + 1}`}
                  >
                    <details>
                      <summary className="cursor-pointer text-muted">
                        {`Other moves tried at step ${i + 1} (${s.menu.length})`}
                      </summary>
                      <ul className="mt-1 space-y-0.5 pl-4">
                        {s.menu.map((c, j) => (
                          <li key={j} className="[overflow-wrap:anywhere]">
                            {`${clean(c.mv.node)}: ${changeWords(c.mv)}, ${price(c.price_micro_usd_hr)}, `}
                            <span className={c.holds ? "text-positive" : "text-negative"}>
                              {c.holds ? "holds" : "does not hold"}
                            </span>
                            {` (${effectWords(c.effect)})`}
                          </li>
                        ))}
                      </ul>
                    </details>
                  </td>
                </tr>
              ) : null}
            </Fragment>
          ))}
        </tbody>
      </table>
    </div>
  );
}
