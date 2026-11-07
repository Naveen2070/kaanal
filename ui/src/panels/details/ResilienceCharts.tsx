// The Results sections for retries and circuit breakers (step 7f): per wire, how its calls ended as
// one stacked bar with the numbers beside it; per breaker, its state over the run as bands. Both
// read lists the run summary carries only when a wire has a retry scope or a breaker changed state,
// so a system without either shows neither section. Built from the tokens already in the app: 2 px
// seams, text beside the bars rather than on them, a sentence for screen readers.

import type { BreakerPhase, BreakerSummary, RetrySummary } from "../../data/api/types";
import { fmtMs } from "../../canvas/metrics";
import { Section } from "./Field";
import { bandsOf, funnelOf } from "./resilienceData";

const FUNNEL_FILL = {
  first: "bg-positive",
  recovered: "bg-accent",
  gave_up: "bg-negative",
  failed: "bg-warn-fill",
} as const;

const BAND_FILL: Record<BreakerPhase, string> = {
  closed: "bg-positive/40",
  half_open: "bg-warn-fill",
  open: "bg-negative",
};

const PHASE_WORD: Record<BreakerPhase, string> = {
  closed: "closed",
  half_open: "half open",
  open: "open",
};

const pctOf = (n: number, whole: number) => (whole > 0 ? Math.round((n / whole) * 100) : 0);
const nf = (n: number) => n.toLocaleString("en-US");

export function RetryFunnel({ retries }: { retries: RetrySummary[] }) {
  if (retries.length === 0) return null;
  return (
    <Section title="How calls ended" help="funnel">
      <ul className="m-0 list-none space-y-3 p-0 text-xs">
        {retries.map((r) => {
          const segs = funnelOf(r);
          const shown = segs.filter((s) => s.count > 0);
          const alt = `${r.from} to ${r.to}: ${segs.map((s) => `${nf(s.count)} ${s.label.toLowerCase()}`).join(", ")}`;
          return (
            <li key={`${r.from}>${r.to}`}>
              <div className="mb-1 flex items-baseline gap-2">
                <span className="min-w-0 flex-1 truncate font-medium">
                  {r.from} → {r.to}
                </span>
                <span className="tabular-nums text-muted">{nf(r.calls)} calls</span>
              </div>
              <figure aria-label={alt} className="m-0">
                <div aria-hidden className="flex h-3 gap-0.5 overflow-hidden rounded-sm">
                  {shown.map((s) => (
                    <div
                      key={s.key}
                      className={FUNNEL_FILL[s.key]}
                      style={{ width: `${(s.count / r.calls) * 100}%` }}
                    />
                  ))}
                </div>
              </figure>
              <ul className="m-0 mt-1 list-none space-y-0.5 p-0">
                {segs.map((s) => (
                  <li key={s.key} className="flex items-center gap-1.5">
                    <span
                      aria-hidden
                      className={`size-2 shrink-0 rounded-sm ${FUNNEL_FILL[s.key]}`}
                    />
                    <span className="min-w-0 flex-1 truncate">{s.label}</span>
                    <span className="tabular-nums text-muted">
                      {nf(s.count)} · {pctOf(s.count, r.calls)}%
                    </span>
                  </li>
                ))}
              </ul>
              <p className="m-0 mt-1 text-[11px] text-muted">
                {nf(r.retries)} retries in all, {Number((r.retries / r.calls).toFixed(1))} for every
                call.
              </p>
            </li>
          );
        })}
      </ul>
    </Section>
  );
}

export function BreakerTimeline({
  breakers,
  windowS,
}: {
  breakers: BreakerSummary[];
  windowS: number;
}) {
  if (breakers.length === 0) return null;
  const windowMs = windowS * 1000;
  return (
    <Section title="Circuit breakers" help="breaker-bands">
      <ul className="m-0 list-none space-y-3 p-0 text-xs">
        {breakers.map((b) => {
          const bands = bandsOf(b, windowMs);
          const name = `${b.from} → ${b.to}${b.replica > 0 ? ` (copy ${b.replica})` : ""}`;
          const alt = `${name}: ${bands
            .map((x) => `${PHASE_WORD[x.state]} from ${fmtMs(x.from)} to ${fmtMs(x.to)}`)
            .join(", ")}`;
          return (
            <li key={`${b.from}>${b.to}#${b.replica}`}>
              <div className="mb-1 flex items-baseline gap-2">
                <span className="min-w-0 flex-1 truncate font-medium">{name}</span>
                <span className="tabular-nums text-muted">
                  open {fmtMs(b.open_ms)} of {fmtMs(windowMs)} · {pctOf(b.open_ms, windowMs)}%
                </span>
              </div>
              <figure aria-label={alt} className="m-0">
                <div aria-hidden className="flex h-3 gap-px overflow-hidden rounded-sm">
                  {bands.map((x, i) => (
                    <div
                      key={i}
                      className={BAND_FILL[x.state]}
                      style={{ width: `${((x.to - x.from) / windowMs) * 100}%` }}
                    />
                  ))}
                </div>
              </figure>
              <div aria-hidden className="mt-0.5 flex justify-between text-[11px] text-muted">
                <span>0 s</span>
                <span>{fmtMs(windowMs)}</span>
              </div>
            </li>
          );
        })}
      </ul>
      <ul className="m-0 mt-1 flex list-none flex-wrap gap-x-3 gap-y-0.5 p-0 text-[11px] text-muted">
        {(["closed", "half_open", "open"] as const).map((p) => (
          <li key={p} className="flex items-center gap-1">
            <span aria-hidden className={`size-2 rounded-sm ${BAND_FILL[p]}`} />
            {PHASE_WORD[p]}
          </li>
        ))}
      </ul>
    </Section>
  );
}
