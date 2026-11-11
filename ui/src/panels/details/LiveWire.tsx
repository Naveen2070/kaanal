// Three small figures under a wire's header while Studio is live: how it did over the last seconds,
// from the finished requests of the live run. Crossings a second, the mean time on the wire and the
// mean wait to get onto it (waiting shows where a wire is shared; on a plain wire it is zero).

import { useLive } from "../../data/store/live";
import { wireFigures } from "../../data/store/liveMath";
import { isLive, useMode } from "../../data/store/mode";
import { fmtMs, twoSignificant } from "../../canvas/metrics";
import { Chip } from "./EffectStrip";

export function LiveWire({ index }: { index: number }) {
  const live = isLive(useMode((m) => m.mode));
  const ticks = useLive((s) => s.ticks);
  if (!live || ticks.length === 0) return null;
  const f = wireFigures(ticks, index);
  if (f.meanWireMs === null) return null;
  return (
    <section aria-label="Live figures for this wire" className="mb-3">
      <div className="grid grid-cols-3 gap-1.5">
        <Chip label="Crossings" value={`${twoSignificant(f.perS)}/s`} />
        <Chip label="On the wire" value={fmtMs(f.meanWireMs)} />
        <Chip label="Waited to get on" value={fmtMs(f.meanWaitMs ?? 0)} />
      </div>
      <p className="m-0 mt-1 text-[11px] text-muted">
        Live: the last 5 seconds, a mean per crossing.
      </p>
    </section>
  );
}
