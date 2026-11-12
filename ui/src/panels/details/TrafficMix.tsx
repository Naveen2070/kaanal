// The traffic mix, with nothing selected: what share of the offered load each endpoint gets, and
// what that comes to per second, as one bar and a row each. The weight is editable in place (a
// weight is relative: 4 and 1 is 80% and 20%), so a mix can be tuned without the full builder.

import { laneColour } from "../../canvas/lanes";
import { useDraft } from "../../data/store/draft";
import { activeProfile, mixOf, setWeight } from "../traffic/model";
import { NumberField } from "./Field";
import { twoSignificant } from "../../canvas/metrics";
import { useMode } from "../../data/store/mode";
import { EditResponse, useEditedRoutes } from "./EditResponse";

export function TrafficMix() {
  const doc = useDraft((s) => s.view?.doc);
  const patch = useDraft((s) => s.patch);
  const profile = activeProfile(doc);
  const rows = mixOf(doc, profile);
  const admin = useMode((s) => s.mode === "admin");
  const { edited, reload } = useEditedRoutes(admin);
  if (rows.length === 0) return null;
  return (
    <section className="mt-4 border-t border-rule pt-3" aria-label="Traffic mix">
      <h3 className="m-0 mb-1 text-xs font-medium text-muted">Traffic mix</h3>
      <div aria-hidden className="mb-2 flex h-2 overflow-hidden rounded-full bg-well">
        {rows.map((r, i) => (
          <span key={r.index} style={{ width: `${r.share * 100}%`, background: laneColour(i) }} />
        ))}
      </div>
      <ul className="m-0 list-none p-0">
        {rows.map((r, i) => (
          <li key={r.index} className="mb-1">
            <div className="flex items-baseline justify-between gap-2">
              <span className="flex min-w-0 items-center gap-1.5">
                <span
                  aria-hidden
                  className="size-2 shrink-0 rounded-full"
                  style={{ background: laneColour(i) }}
                />
                <span className="min-w-0 truncate font-medium">{r.key}</span>
              </span>
              <span className="flex shrink-0 items-center gap-2">
                <span className="tabular-nums">{twoSignificant(r.share * 100)}%</span>
                <EditResponse route={r.key} edited={edited.has(r.key)} onClose={reload} />
              </span>
            </div>
            <NumberField
              label={`Weight of ${r.key}`}
              hint={
                r.perS === null
                  ? "Its part of the mix, against the other weights."
                  : `About ${twoSignificant(r.perS)} requests a second at the current rate.`
              }
              value={r.weight}
              min={0.0001}
              step="any"
              onCommit={(n) => patch([setWeight(profile, r.index, n)])}
            />
          </li>
        ))}
      </ul>
    </section>
  );
}
