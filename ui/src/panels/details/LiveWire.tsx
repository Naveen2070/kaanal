// What a wire is doing while Studio is live, from the finished requests and the engine's own counts
// over the last seconds: crossings a second, the mean time on the wire and the mean wait to get onto
// it (waiting shows where a wire is shared), how full a wire with a bandwidth is, what became of its
// retried calls, and what its circuit breakers did over the last minute.

import { useLive } from "../../data/store/live";
import {
  breakerBands,
  wireFigures,
  type BreakerBand,
  type BreakerPhase,
} from "../../data/store/liveMath";
import { isLive, useMode } from "../../data/store/mode";
import { fmtMs, percent, twoSignificant } from "../../canvas/metrics";
import { Chip } from "./EffectStrip";

const PHASE_WORD: Record<BreakerPhase, string> = {
  closed: "closed",
  half_open: "half open (testing)",
  open: "open (refusing calls)",
};

const PHASE_FILL: Record<BreakerPhase, string> = {
  closed: "bg-ok-fill",
  half_open: "bg-warn-fill",
  open: "bg-bad-fill",
};

/** The breaker's last minute as one bar: a run of equal states is one segment. */
function BreakerBar({ band }: { band: BreakerBand }) {
  const runs: { phase: BreakerPhase; n: number }[] = [];
  for (const p of band.phases) {
    const last = runs.at(-1);
    if (last?.phase === p) last.n++;
    else runs.push({ phase: p, n: 1 });
  }
  const label = `Breaker, copy ${band.replica}: ${PHASE_WORD[band.now]} now; not closed for ${twoSignificant(band.openS)} s of the last minute`;
  return (
    <div className="mt-1.5">
      <div className="flex justify-between text-[11px] text-muted">
        <span>Breaker (copy {band.replica})</span>
        <span className={band.now === "closed" ? "" : "font-semibold text-text"}>
          {PHASE_WORD[band.now]}
        </span>
      </div>
      <div aria-hidden className="mt-0.5 flex h-2 overflow-hidden rounded-full bg-well">
        {runs.map((r, i) => (
          <div
            key={i}
            className={PHASE_FILL[r.phase]}
            style={{ width: `${(r.n / band.phases.length) * 100}%` }}
          />
        ))}
      </div>
      <span className="sr-only">{label}</span>
    </div>
  );
}

export function LiveWire({ index }: { index: number }) {
  const live = isLive(useMode((m) => m.mode));
  const ticks = useLive((s) => s.ticks);
  if (!live || ticks.length === 0) return null;
  const f = wireFigures(ticks, index);
  const bands = breakerBands(ticks, index);
  if (f.meanWireMs === null && bands.length === 0) return null;
  return (
    <section aria-label="Live figures for this wire" className="mb-3">
      {f.meanWireMs !== null ? (
        <div className="grid grid-cols-3 gap-1.5">
          <Chip label="Crossings" value={`${twoSignificant(f.perS)}/s`} />
          <Chip label="On the wire" value={fmtMs(f.meanWireMs)} />
          <Chip label="Waited to get on" value={fmtMs(f.meanWaitMs ?? 0)} />
          {f.p50Ms !== null && f.p99Ms !== null ? (
            <Chip label="Crossing p50 / p99" value={`${fmtMs(f.p50Ms)} / ${fmtMs(f.p99Ms)}`} />
          ) : null}
          {f.load !== null ? <Chip label="Wire full" value={percent(f.load)} /> : null}
          {f.funnel ? (
            <>
              <Chip label="Retried calls" value={f.funnel.retried.toLocaleString("en-US")} />
              <Chip
                label="Recovered"
                value={`${f.funnel.recovered.toLocaleString("en-US")} (gave up ${f.funnel.gaveUp.toLocaleString("en-US")})`}
              />
            </>
          ) : null}
        </div>
      ) : null}
      {bands.map((b) => (
        <BreakerBar key={b.replica} band={b} />
      ))}
      <p className="m-0 mt-1 text-[11px] text-muted">
        Live: the last 5 seconds, a mean per crossing; p50 and p99 are per request (wait included)
        {f.funnel ? "; gave up is retried minus recovered, so a call still retrying counts" : ""}.
      </p>
    </section>
  );
}
