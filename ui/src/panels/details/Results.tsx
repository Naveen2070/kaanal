// The Results tab (spec 7.2): the report label and its warnings, what breaks first (components and
// wires), a heatmap for one component or one wire (latency, or time on the wire), per-instance and
// per-link figures, errors by kind.

import { useId, useState } from "react";
import { useRun } from "../../data/store/run";
import { useSelected } from "../../data/store/view";
import { Heatmap } from "./Heatmap";
import { Section } from "./Field";
import { fmtMs, rankedByPeak, twoSignificant } from "../../canvas/metrics";
import { ResultsSkeleton, RANK_ROW } from "./Skeletons";

/** Starts the value of a wire in the picker, so it cannot be a component's name. */
const WIRE = String.fromCharCode(31);

const pct = (x: number | null) => (x === null ? "–" : `${Math.round(x * 100)}%`);

export function Results() {
  const run = useRun((s) => s.latest);
  const [selected] = useSelected();
  // a pick holds only for the selection it was made under: selecting on the canvas moves the heatmap
  // a key is a component's name, or a wire's place in the run's list behind a control character
  const [picked, setPicked] = useState<{ key: string; under: string | null } | null>(null);
  const dimmed = useRun((s) => s.dimmed);
  const status = useRun((s) => s.status);
  const pickId = useId();
  const s = run?.summary;

  if (!s && status === "running") return <ResultsSkeleton />;
  if (!s) {
    return (
      <p className="p-3 text-sm text-muted">
        No finished run yet. The results appear after the first run.
      </p>
    );
  }

  // the heatmap follows the selected component, else the first bottleneck, else the first node
  const ranked = rankedByPeak(s);
  const names = s.nodes.map((n) => n.name);
  const fallback = s.bottlenecks[0] !== undefined ? s.nodes[s.bottlenecks[0]]?.name : names[0];
  const wireKeys = s.links.map((_, k) => `${WIRE}${k}`);
  const keys = [...names, ...wireKeys];
  const pick = picked && picked.under === selected ? picked.key : null;
  const chosenKey =
    [pick, selected, fallback].find(
      (k): k is string => typeof k === "string" && keys.includes(k),
    ) ?? keys[0];
  const link = chosenKey?.startsWith(WIRE) ? s.links[Number(chosenKey.slice(1))] : undefined;
  const node = link ? undefined : s.nodes.find((n) => n.name === chosenKey);

  return (
    <div className="p-3 text-sm">
      {/* always present: a number from this tool is a relative what-if, never a prediction */}
      {dimmed ? (
        <p className="mb-2 text-xs text-warn">
          Out of date: the newest run could not finish, so these are the last figures that did.
        </p>
      ) : null}
      <p className="mb-2 text-xs font-medium">
        {run ? `Run ${run.id.replace(/^s/, "")} · ` : ""}A what-if, not a prediction
      </p>
      {s.warnings.length > 0 ? (
        <ul className="mb-3 space-y-1 rounded-lg border border-warn p-2 text-xs text-warn">
          {s.warnings.map((w, i) => (
            <li key={i}>{w}</li>
          ))}
        </ul>
      ) : null}

      <Section title="What breaks first">
        {ranked.length === 0 ? (
          <p className="text-xs text-muted">No traffic reached any component or wire.</p>
        ) : (
          <ol className="text-xs">
            {ranked.map((r, i) => (
              <li key={r.name} className={`flex flex-col justify-center ${RANK_ROW}`}>
                <div className="flex items-baseline gap-2">
                  <span
                    title={r.wire ? `${r.name} (a wire)` : r.name}
                    className="min-w-0 flex-1 truncate font-medium"
                  >
                    {r.name}
                  </span>
                  <span className="tabular-nums text-muted">peak {pct(r.peak)}</span>
                </div>
                {/* two measures stack in one row: the name over its figure, the limit under it */}
                <div className="tabular-nums text-[11px] text-muted">
                  {r.wire ? "Wire · " : ""}
                  {r.saturatesAtReqS !== null
                    ? `saturates at ${twoSignificant(r.saturatesAtReqS)} req/s`
                    : "no saturation point"}
                </div>
                <div aria-hidden className="h-1 rounded-full bg-well">
                  <div
                    className={`h-1 rounded-full ${i === 0 ? "bg-bad-fill" : "bg-text"}`}
                    style={{ width: `${Math.min(r.peak, 1) * 100}%` }}
                  />
                </div>
              </li>
            ))}
          </ol>
        )}
      </Section>

      <Section title="Latency over the run">
        {node || link ? (
          <>
            <label htmlFor={pickId} className="mb-1 block text-xs">
              {s.links.length > 0 ? "Component or wire" : "Component"}{" "}
              <select
                id={pickId}
                value={chosenKey}
                onChange={(e) => setPicked({ key: e.currentTarget.value, under: selected })}
                className="max-w-full rounded border border-control bg-sheet text-text px-1"
              >
                <optgroup label="Components">
                  {names.map((n) => (
                    <option key={n} value={n}>
                      {n}
                    </option>
                  ))}
                </optgroup>
                {s.links.length > 0 ? (
                  <optgroup label="Wires">
                    {s.links.map((l, k) => (
                      <option key={wireKeys[k]} value={wireKeys[k]}>
                        {l.from} → {l.to}
                      </option>
                    ))}
                  </optgroup>
                ) : null}
              </select>
            </label>
            {link ? (
              <Heatmap
                wire
                name={`${link.from} → ${link.to}`}
                series={link.series}
                buckets={s.latency.buckets}
              />
            ) : node ? (
              <Heatmap name={node.name} series={node.series} buckets={s.latency.buckets} />
            ) : null}
          </>
        ) : (
          <p className="text-xs text-muted">This run has no components.</p>
        )}
      </Section>

      {node && node.replicas.length > 1 ? (
        <Section title={`Instances of ${node.name}`}>
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left">
                <th>Instance</th>
                <th>Visits/s</th>
                <th>Busy</th>
                <th>Wait</th>
              </tr>
            </thead>
            <tbody>
              {node.replicas.map((r) => (
                <tr key={r.replica}>
                  <td>{r.replica}</td>
                  <td>{twoSignificant(r.visits_per_s)}</td>
                  <td>{pct(r.utilization)}</td>
                  <td>{twoSignificant(r.mean_wait_ms)} ms</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
      ) : null}

      {s.links.length > 0 ? (
        <Section title="Links">
          <ul className="space-y-1 text-xs">
            {s.links.map((l) => (
              <li key={`${l.from}>${l.to}`}>
                {l.from} to {l.to}: {pct(l.utilization)} busy, {fmtMs(l.mean_wait_ms)} wait,{" "}
                {l.saturated ? "saturated" : "ok"}
                {l.saturates_at_req_s !== null
                  ? `, fills at ${twoSignificant(l.saturates_at_req_s)} req/s`
                  : ""}
              </li>
            ))}
          </ul>
        </Section>
      ) : null}

      <Section title="Errors by kind">
        {Object.keys(s.errors).length === 0 ? (
          <p className="text-xs tabular-nums">0</p>
        ) : (
          <ul className="text-xs">
            {Object.entries(s.errors).map(([kind, n]) => (
              <li key={kind}>
                {kind}: {n}
              </li>
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}
