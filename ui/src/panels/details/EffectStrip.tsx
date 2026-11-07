// Three small figures under a wire's header: how the last run went over this wire. They answer "what
// did my setting do" without leaving Controls. Only a wire that carried a retry scope has them (the
// run summary counts nothing else per wire), so a plain wire shows nothing here. Read from the
// run's `retries` and `breakers` lists; the figures change after the auto re-run.

import { useRun } from "../../data/store/run";
import type { EdgeDoc } from "../../data/api/types";
import { wireEffects } from "./resilienceData";

export function Chip({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-lg bg-raised px-2 py-1.5">
      <span className="block truncate text-xs text-muted">{label}</span>
      <b className="block font-mono text-sm tabular-nums">{value}</b>
    </div>
  );
}

export function EffectStrip({ e }: { e: EdgeDoc }) {
  const s = useRun((x) => x.latest?.summary);
  const [from, to] = [String(e.from), String(e.to)];
  const r = s?.retries?.find((x) => x.from === from && x.to === to);
  if (!s || !r) return null;
  const b = s.breakers?.find((x) => x.from === from && x.to === to);
  const fx = wireEffects(r, b, s.window_s * 1000);
  if (!fx) return null;
  return (
    <section aria-label="Last run over this wire" className="mb-3">
      <div className="grid grid-cols-3 gap-1.5">
        <Chip label="Calls failed" value={`${fx.failedPct}%`} />
        <Chip
          label="Recovered"
          value={fx.recoveredPct === null ? "none retried" : `${fx.recoveredPct}%`}
        />
        <Chip
          label={fx.openPct === null ? "Retries per call" : "Breaker open"}
          value={fx.openPct === null ? `${fx.retriesPerCall}×` : `${fx.openPct}%`}
        />
      </div>
      <p className="m-0 mt-1 text-[11px] text-muted">
        From the last run: {r.calls.toLocaleString("en-US")} calls over this wire.
      </p>
    </section>
  );
}
