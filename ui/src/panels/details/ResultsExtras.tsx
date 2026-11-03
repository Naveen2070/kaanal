// The parts of Results that read a run rather than draw it: how it differs from the run before, a p99
// target, a rough plan for a higher rate, and the report as Markdown. Each is worked out in the
// browser from the run summary (canvas/insights.ts); none asks the server for anything.

import { Download } from "lucide-react";
import { useState } from "react";
import { compareRuns, planFor, reportMarkdown, sloVerdict } from "../../canvas/insights";
import { rankedByPeak } from "../../canvas/metrics";
import type { LinkSummary, Summary } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { usePrevRun } from "../../data/store/run";
import { NumberField, Section } from "./Field";
import { bandwidthKbOf } from "./edgeFields";
import { useSlo } from "./slo";
import { TIME_UNITS } from "./units";

/** The bandwidth of a wire as the draft has it, in KB/s; null when it has none. */
export function useBandwidthOf(): (l: LinkSummary) => number | null {
  const edges = useDraft((s) => s.view?.doc?.edges);
  return (l) => {
    const e = edges?.find((x) => String(x.from) === l.from && String(x.to) === l.to);
    return e ? bandwidthKbOf(e) : null;
  };
}

/** "Compared with run 1: p99 11.7 h → 4.1 s". Nothing when there is no earlier run or nothing moved. */
export function CompareLine({ runId, s }: { runId: string | undefined; s: Summary }) {
  const prev = usePrevRun((x) => x.prev);
  if (!prev?.summary || prev.id === runId) return null;
  const lines = compareRuns(prev.summary, s);
  if (lines.length === 0) return null;
  return (
    <p className="mb-2 rounded-lg border border-border p-2 text-xs">
      <span className="text-muted">Compared with run {prev.id.replace(/^s/, "")}: </span>
      {lines.join(" · ")}
    </p>
  );
}

/** The p99 target: a box with a unit, and one sentence saying where the run stands against it. */
export function SloSection({ s }: { s: Summary }) {
  const target = useSlo((x) => x.targetMs);
  const set = useSlo((x) => x.set);
  const verdict = sloVerdict(s.latency.p99_ms, target);
  const over = target !== null && s.latency.p99_ms > target;
  return (
    <Section title="p99 target">
      <NumberField
        label="Hold the system p99 to"
        hint="Kept in this browser. Empty means no target."
        value={target}
        units={TIME_UNITS}
        placeholder="No target"
        min={0.001}
        max={3_600_000}
        step="any"
        onCommit={(n) => set(n)}
      />
      {target !== null ? (
        <button
          type="button"
          onClick={() => set(null)}
          className="mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          Remove the target
        </button>
      ) : null}
      {verdict ? (
        <p className={`text-xs ${over ? "text-negative" : "text-ok"}`}>{verdict}</p>
      ) : null}
    </Section>
  );
}

/** A rough plan for a higher rate: what would need more copies or bandwidth. */
export function PlanSection({ s }: { s: Summary }) {
  const [rate, setRate] = useState("");
  const bandwidthOf = useBandwidthOf();
  const n = Number(rate);
  const rows =
    rate.trim() !== "" && Number.isFinite(n) && n > 0 ? planFor(s, n, bandwidthOf) : null;
  return (
    <Section title="Plan for a rate">
      <label className="mb-1 block text-xs">
        Requests a second{" "}
        <input
          type="number"
          min={1}
          value={rate}
          onChange={(e) => setRate(e.currentTarget.value)}
          className="h-7 w-24 rounded border border-control bg-sheet px-1 text-right text-text"
        />
      </label>
      {rows === null ? (
        <p className="text-xs text-muted">Type a rate to see what would need more.</p>
      ) : rows.length === 0 ? (
        <p className="text-xs">Nothing needs more at {n.toLocaleString("en-US")} req/s.</p>
      ) : (
        <ul className="m-0 list-none space-y-1 p-0 text-xs">
          {rows.map((r) => (
            <li key={r.name}>
              <span className="font-medium">{r.name}</span>: {r.text}
            </li>
          ))}
        </ul>
      )}
      <p className="mt-1 text-[11px] text-muted">
        Rough: today&apos;s run scaled in a straight line, each kept under 70% busy. Run it at that
        rate to check.
      </p>
    </Section>
  );
}

/** Copy the report as Markdown, or save it as a file. */
export function ReportButtons({ runId, s }: { runId: string | undefined; s: Summary }) {
  const [said, setSaid] = useState("");
  const text = () =>
    reportMarkdown(runId ? `Run ${runId.replace(/^s/, "")}` : "Run", s, rankedByPeak(s));
  const btn =
    "flex min-h-7 items-center gap-1 rounded border border-control bg-sheet px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";
  return (
    <div className="mb-3 flex flex-wrap items-center gap-2">
      <button
        type="button"
        className={btn}
        onClick={() => {
          void navigator.clipboard
            ?.writeText(text())
            .then(() => setSaid("Copied"))
            .catch(() => setSaid("Could not copy: use Save as a file"));
        }}
      >
        Copy as Markdown
      </button>
      <button
        type="button"
        className={btn}
        onClick={() => {
          const url = URL.createObjectURL(new Blob([text()], { type: "text/markdown" }));
          const a = document.createElement("a");
          a.href = url;
          a.download = `kaanal-run-${runId ?? "report"}.md`;
          a.click();
          URL.revokeObjectURL(url);
          setSaid("Saved");
        }}
      >
        <Download size={12} aria-hidden />
        Save as a file
      </button>
      <output className="text-xs text-muted">{said}</output>
    </div>
  );
}
