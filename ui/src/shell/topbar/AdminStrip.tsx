// The strip of Admin mode (step 8A): the live strip, plus how much of the traffic is a real
// frontend's and how much Kaanal generated, and a "Real only" switch that turns every figure here
// and on the canvas to the real share.

import { useAdmin } from "../../data/store/admin";
import { useLive } from "../../data/store/live";
import { windowFigures } from "../../data/store/liveMath";
import { twoSignificant } from "../../canvas/metrics";
import { LiveStrip } from "./LiveStrip";

export function AdminStrip() {
  const ticks = useLive((s) => s.ticks);
  const realOnly = useAdmin((s) => s.realOnly);
  const setRealOnly = useAdmin((s) => s.setRealOnly);
  const real = windowFigures(ticks, 1000, "real");
  const generated = windowFigures(ticks, 1000, "generated");
  const seen = ticks.some((t) => t.real !== undefined);
  return (
    <div>
      <LiveStrip src={realOnly ? "real" : "all"} throttle={false} />
      <section
        aria-label="Real and generated requests"
        className="flex flex-wrap items-center gap-x-5 gap-y-1 px-3 pb-2 text-sm"
      >
        {seen ? (
          <dl className="m-0 flex flex-wrap items-baseline gap-x-5 gap-y-1">
            <div className="flex items-baseline gap-1.5">
              <dt className="text-xs text-muted">Real</dt>
              <dd className="m-0 font-mono tabular-nums">{twoSignificant(real.reqPerS)} / s</dd>
            </div>
            <div className="flex items-baseline gap-1.5">
              <dt className="text-xs text-muted">Generated</dt>
              <dd className="m-0 font-mono tabular-nums">
                {twoSignificant(generated.reqPerS)} / s
              </dd>
            </div>
          </dl>
        ) : (
          <p className="m-0 text-xs text-muted">
            No real requests yet. Point your frontend at the mock and they show here.
          </p>
        )}
        <button
          type="button"
          aria-pressed={realOnly}
          onClick={() => setRealOnly(!realOnly)}
          className={`ml-auto h-7 rounded-md border border-control px-2.5 text-xs font-medium focus-visible:outline-2 focus-visible:outline-accent ${realOnly ? "bg-raised text-text" : "bg-sheet text-muted hover:bg-raised"}`}
        >
          Real only
        </button>
      </section>
    </div>
  );
}
