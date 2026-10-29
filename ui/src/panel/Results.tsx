// The Results tab (spec 7.2): the report label and its warnings, a latency heatmap for one
// component, per-instance and per-link figures, errors by kind, and the bottleneck order the
// server computed.

import { useId, useState } from "react";
import { useRun } from "../store/run";
import { useSelected } from "../store/view";
import { Heatmap } from "./Heatmap";
import { Section } from "./Field";
import { twoSignificant } from "../canvas/metrics";

const pct = (x: number | null) => (x === null ? "–" : `${Math.round(x * 100)}%`);

export function Results() {
  const run = useRun((s) => s.latest);
  const [selected] = useSelected();
  // a pick holds only for the selection it was made under: selecting on the canvas moves the heatmap
  const [picked, setPicked] = useState<{ name: string; under: string | null } | null>(null);
  const dimmed = useRun((s) => s.dimmed);
  const pickId = useId();
  const s = run?.summary;

  if (!s) {
    return (
      <p className="p-3 text-sm text-neutral-500">
        No finished run yet. The results appear after the first run.
      </p>
    );
  }

  // the heatmap follows the selected component, else the first bottleneck, else the first node
  const names = s.nodes.map((n) => n.name);
  const fallback = s.bottlenecks[0] !== undefined ? s.nodes[s.bottlenecks[0]]?.name : names[0];
  const pick = picked && picked.under === selected ? picked.name : null;
  const chosen = [pick, selected, fallback].find((n) => n && names.includes(n)) ?? names[0];
  const node = s.nodes.find((n) => n.name === chosen);

  return (
    <div className="p-3 text-sm">
      {/* always present: a number from this tool is a relative what-if, never a prediction */}
      {dimmed ? (
        <p className="mb-2 text-xs text-amber-700">
          Out of date: the newest run could not finish, so these are the last figures that did.
        </p>
      ) : null}
      <p className="mb-2 text-xs font-medium text-neutral-700">{s.label}</p>
      {s.warnings.length > 0 ? (
        <ul className="mb-2 space-y-1 rounded border border-amber-400 bg-amber-50 p-2 text-xs text-amber-900">
          {s.warnings.map((w, i) => (
            <li key={i}>{w}</li>
          ))}
        </ul>
      ) : null}

      <Section title="Latency over the run">
        {node ? (
          <>
            <label htmlFor={pickId} className="mb-1 block text-xs">
              Component{" "}
              <select
                id={pickId}
                value={node.name}
                onChange={(e) => setPicked({ name: e.currentTarget.value, under: selected })}
                className="rounded border px-1"
              >
                {names.map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
            <Heatmap name={node.name} series={node.series} buckets={s.latency.buckets} />
          </>
        ) : (
          <p className="text-xs text-neutral-500">This run has no components.</p>
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
                {l.from} to {l.to}: {pct(l.utilization)} busy, {twoSignificant(l.mean_wait_ms)} ms
                wait, {l.saturated ? "saturated" : "ok"}
              </li>
            ))}
          </ul>
        </Section>
      ) : null}

      <Section title="Errors by kind">
        {Object.keys(s.errors).length === 0 ? (
          <p className="text-xs text-neutral-500">None in this run.</p>
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

      <Section title="Where it breaks first">
        {s.bottlenecks.length === 0 ? (
          <p className="text-xs text-neutral-500">No traffic reached any component.</p>
        ) : (
          <ol className="list-decimal pl-4 text-xs">
            {s.bottlenecks.map((i) => {
              const n = s.nodes[i];
              return n ? (
                <li key={i}>
                  {n.name}
                  {n.offered_load !== null ? ` (${pct(n.offered_load)} offered load)` : ""}
                </li>
              ) : null;
            })}
          </ol>
        )}
      </Section>
    </div>
  );
}
