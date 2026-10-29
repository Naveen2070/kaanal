// The top bar: the offered-load slider (the traffic profile's rate), the system figures of the
// newest run, a Run button, and a plain line saying what the loop is doing. A slider change is an
// edit of the draft like any other, so it goes through the same auto re-run.

import { useId, useState } from "react";
import { LIMITS, setRate } from "../panel/fields";
import { useFirstRun } from "../first-run/store";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { SaveFlow } from "./SaveFlow";

const SLIDER_KEYS = new Set([
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "ArrowDown",
  "PageUp",
  "PageDown",
  "Home",
  "End",
]);

function dash(n: number | undefined, digits = 1): string {
  return n === undefined ? "–" : n.toFixed(digits);
}

export function TopBar({ onRun }: { onRun: () => void }) {
  const view = useDraft((s) => s.view);
  const patch = useDraft((s) => s.patch);
  const replayTour = useFirstRun((s) => s.replayTour);
  const { status, latest, dimmed, lastError } = useRun();
  const id = useId();
  /** The slider while it is being moved, before it is committed. */
  const [dragging, setDragging] = useState<number | null>(null);

  const traffic = view?.doc?.traffic as Record<string, { rps?: unknown }> | undefined;
  const profile = traffic ? Object.keys(traffic)[0] : undefined;
  const rps = profile !== undefined ? traffic?.[profile]?.rps : undefined;
  const rate = typeof rps === "number" ? rps : null;
  const shown = dragging ?? rate ?? 1;
  const commit = (v: number) => {
    setDragging(null);
    if (profile !== undefined && Number.isFinite(v) && v !== rate) {
      void patch([setRate(profile, Math.min(Math.max(v, LIMITS.rate.min), LIMITS.rate.max))]);
    }
  };
  const s = latest?.summary ?? undefined;
  const goodput = s && s.window_s > 0 ? (s.requests - s.failed) / s.window_s : undefined;
  const dropped = s?.nodes.reduce((n, x) => n + x.limited + x.overflowed, 0);

  const line =
    status === "running"
      ? "Running..."
      : lastError
        ? lastError
        : status === "done"
          ? "Up to date"
          : "Edit anything: it re-runs after a short pause.";

  return (
    <header className="flex flex-wrap items-center gap-4 border-b px-3 py-1 text-sm">
      <div className="flex items-center gap-2">
        <label htmlFor={id}>Offered load</label>
        <input
          id={id}
          type="range"
          // no `key`: remounting after each commit would drop keyboard focus. `any` and a low
          // `min` so a rate such as 0.5 or 12.5 is shown as it is and never snapped by tabbing past
          min={Math.min(1, rate ?? 1)}
          max={Math.min(LIMITS.rate.max, Math.max(100, shown * 3))}
          step="any"
          disabled={profile === undefined || rate === null}
          value={shown}
          onChange={(e) => setDragging(Number(e.currentTarget.value))}
          // one patch when the hand lets go or an arrow/page/home/end key is released, not one per
          // pixel of the drag and not for a Tab that merely lands here
          onPointerUp={(e) => commit(Number(e.currentTarget.value))}
          onKeyUp={(e) => {
            if (SLIDER_KEYS.has(e.key)) commit(Number(e.currentTarget.value));
          }}
          aria-valuetext={rate === null ? "no traffic profile" : `${shown} requests per second`}
        />
        <span className="tabular-nums">{rate === null ? "–" : `${shown}/s`}</span>
      </div>
      <dl className={`flex gap-4 ${dimmed ? "opacity-60" : ""}`}>
        <div>
          <dt className="inline text-neutral-500">P99 </dt>
          <dd className="inline">{dash(s?.latency.p99_ms)} ms</dd>
        </div>
        <div>
          <dt className="inline text-neutral-500">Goodput </dt>
          <dd className="inline">{dash(goodput, 0)}/s</dd>
        </div>
        <div>
          <dt className="inline text-neutral-500">Errors </dt>
          <dd className="inline">{s?.failed ?? "–"}</dd>
        </div>
        <div>
          <dt className="inline text-neutral-500">Dropped </dt>
          <dd className="inline">{dropped ?? "–"}</dd>
        </div>
      </dl>
      <button type="button" data-tour="run" onClick={onRun} className="rounded border px-2">
        Run
      </button>
      <output className={`text-xs ${lastError ? "text-red-700" : "text-neutral-500"}`}>
        {line}
      </output>
      <SaveFlow />
      <button
        type="button"
        aria-label="Show the tour"
        title="Show the tour"
        onClick={replayTour}
        className="ml-auto rounded-full border px-2"
      >
        ?
      </button>
    </header>
  );
}
