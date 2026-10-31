// The top bar: the offered-load slider (the traffic profile's rate), the system figures of the
// newest run, a Run button, and a plain line saying what the loop is doing. A slider change is an
// edit of the draft like any other, so it goes through the same auto re-run.

import { useId, useState } from "react";
import type { Summary } from "../api/types";
import { LIMITS, setRate } from "../panel/fields";
import { useFirstRun } from "../first-run/store";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { Mark } from "../brand/Mark";
import { SaveFlow } from "./SaveFlow";
import { ThemeToggle } from "./ThemeToggle";

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

/** The system bottleneck saturation threshold (in req/s), or null when none is measured. */
export function systemThreshold(summary: Summary | undefined): number | null {
  if (!summary || !summary.nodes || summary.nodes.length === 0) return null;
  const first = summary.bottlenecks?.[0];
  if (first !== undefined) {
    const bNode = summary.nodes[first];
    if (bNode && typeof bNode.saturates_at_req_s === "number" && bNode.saturates_at_req_s > 0) {
      return bNode.saturates_at_req_s;
    }
  }
  const all = summary.nodes
    .map((n) => n.saturates_at_req_s)
    .filter((x): x is number => typeof x === "number" && x > 0 && Number.isFinite(x));
  return all.length > 0 ? Math.min(...all) : null;
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

  // The throttle in the UI is strictly integer: no float
  const shown = dragging !== null ? Math.round(dragging) : rate !== null ? Math.round(rate) : 1;

  const commit = (v: number) => {
    const intVal = Math.round(v);
    setDragging(null);
    const currentInt = rate !== null ? Math.round(rate) : null;
    if (profile !== undefined && Number.isFinite(intVal) && intVal !== currentInt) {
      void patch([setRate(profile, Math.min(Math.max(intVal, 1), LIMITS.rate.max))]);
    }
  };

  const s = latest?.summary ?? undefined;
  const goodput = s && s.window_s > 0 ? (s.requests - s.failed) / s.window_s : undefined;
  const dropped = s?.nodes.reduce((n, x) => n + x.limited + x.overflowed, 0);

  const rawThreshold = systemThreshold(s);
  const threshold = rawThreshold !== null ? Math.round(rawThreshold) : null;

  // Scale the slider max a few thousand above the threshold if known, else standard scale
  const min = 1;
  const max = Math.min(
    LIMITS.rate.max,
    Math.max(
      threshold !== null ? Math.ceil((threshold + 2000) / 1000) * 1000 : 100,
      shown * 3,
      100,
    ),
  );

  const thresholdPercent =
    threshold !== null && threshold >= min && threshold <= max
      ? ((threshold - min) / (max - min)) * 100
      : null;

  const line =
    status === "running"
      ? "Running..."
      : lastError
        ? lastError
        : status === "done"
          ? "Up to date"
          : "Edit anything: it re-runs after a short pause.";

  return (
    <header className="flex min-h-11 flex-wrap items-center gap-4 px-4 py-1 text-sm">
      <h1 className="flex items-center gap-2 text-base font-semibold">
        <Mark size={24} className="text-accent" />
        {view?.project_name ?? "Kaanal Studio"}
      </h1>
      <div className="flex items-center gap-2">
        <label htmlFor={id}>Offered load</label>
        <div className="relative flex items-center">
          <input
            id={id}
            type="range"
            min={min}
            max={max}
            step={1}
            disabled={profile === undefined || rate === null}
            value={shown}
            onChange={(e) => setDragging(Math.round(Number(e.currentTarget.value)))}
            onPointerUp={(e) => commit(Math.round(Number(e.currentTarget.value)))}
            onKeyUp={(e) => {
              if (SLIDER_KEYS.has(e.key)) commit(Math.round(Number(e.currentTarget.value)));
            }}
            aria-valuetext={
              rate === null
                ? "no traffic profile"
                : `${shown} requests per second${
                    threshold !== null ? `, threshold limit ${threshold} per second` : ""
                  }`
            }
          />
          {thresholdPercent !== null && (
            <div
              aria-hidden="true"
              title={`Threshold limit: ${threshold}/s`}
              className="pointer-events-none absolute inset-y-0 flex items-center"
              style={{ left: `${thresholdPercent}%`, transform: "translateX(-50%)" }}
            >
              <span className="font-mono text-xs font-bold text-red-600 select-none">|</span>
            </div>
          )}
        </div>
        <span className="tabular-nums">{rate === null ? "–" : `${shown}/s`}</span>
        {threshold !== null && (
          <span className="text-xs text-neutral-500 tabular-nums" title="Threshold limit">
            (limit {threshold}/s)
          </span>
        )}
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
        <div>
          <dt className="inline text-neutral-500">Max threshold </dt>
          <dd className="inline">{threshold !== null ? `${threshold}/s` : "–"}</dd>
        </div>
      </dl>
      <button type="button" data-tour="run" onClick={onRun} className="rounded border px-2">
        Run
      </button>
      <output className={`text-xs ${lastError ? "text-red-700" : "text-neutral-500"}`}>
        {line}
      </output>
      <SaveFlow />
      <div className="ml-auto flex items-center gap-1">
        <ThemeToggle />
        <button
          type="button"
          aria-label="Show the tour"
          title="Show the tour"
          onClick={replayTour}
          className="h-8 min-w-8 rounded-full border border-control px-2"
        >
          ?
        </button>
      </div>
    </header>
  );
}
