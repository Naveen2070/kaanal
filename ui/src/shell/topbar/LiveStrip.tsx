// The key figures of the live run, in the place of the batch strip while Studio is live (step 8L):
// what is arriving, what was dropped this second, p50 and p99 over the last seconds, the busiest
// component, and the generator (state, Pause or Resume, the offered rate). The figures update four
// times a second; a quiet live region says the same in a sentence every few seconds, so a screen
// reader is not interrupted by every tick.

import { Pause, Play, Workflow } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../../data/api/client";
import { useDraft } from "../../data/store/draft";
import { useLive, type LiveStatus } from "../../data/store/live";
import { windowFigures, type Source } from "../../data/store/liveMath";
import { pauseLive, resumeLive } from "../../data/store/liveRuntime";
import { liveFeed } from "../../canvas/liveFeed";
import { rpsFromSlider, SLIDER_MAX, sliderFromRps } from "../../canvas/logScale";
import { openTrafficBuilder } from "../../panels/traffic/store";
import { fmtMs, twoSignificant } from "../../canvas/metrics";
import { p99WarningFor } from "../../canvas/p99Warning";
import { notify } from "../toast/toasts";

const STATE_WORDS: Record<LiveStatus, string> = {
  off: "Not running",
  connecting: "Connecting",
  running: "Running at real speed",
  paused: "Paused",
  "hidden-paused": "Paused: the page was hidden",
};

const pct = (n: number) => `${Math.round(n * 100)}%`;

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

/** `throttle` is the offered-load slider; Admin hides it (a real frontend sets the load) but keeps Build traffic. */
export function LiveStrip({
  src = "all",
  throttle = true,
}: { src?: Source; throttle?: boolean } = {}) {
  const ticks = useLive((s) => s.ticks);
  const status = useLive((s) => s.status);
  const hello = useLive((s) => s.hello);
  const doc = useDraft((s) => s.view?.doc ?? null);
  const fig = windowFigures(ticks, 1000, src);
  const slow = windowFigures(ticks, 10_000, src);
  const busiest = liveFeed(ticks, doc, hello?.nodes, src === "real" ? "real" : "all")
    .nodes.filter((n) => n.utilization !== null)
    .sort((a, b) => (b.utilization ?? 0) - (a.utilization ?? 0))[0];
  const queue = busiest?.series.queue.at(-1) ?? 0;
  // the nearest limit by the utilization law: an estimate, so it says "about"
  const firstLimit = liveFeed(ticks, doc, hello?.nodes, src === "real" ? "real" : "all")
    .nodes.filter((n) => n.saturates_at_req_s !== null)
    .sort((a, b) => (a.saturates_at_req_s ?? 0) - (b.saturates_at_req_s ?? 0))[0];
  const dropping = fig.droppedPerS > 0;
  // every tick of the window, not the last second: what failed since the run began
  const failedTotal = ticks.reduce(
    (n, t) => n + (src === "real" ? (t.real?.failed ?? 0) : t.failed),
    0,
  );
  const tooSlow = slow.p99Ms === null ? null : p99WarningFor(slow.p99Ms, doc);
  const sentence =
    `${STATE_WORDS[status]}. ${twoSignificant(fig.reqPerS)} requests a second, ` +
    `${twoSignificant(fig.droppedPerS)} dropped a second` +
    (slow.p99Ms === null ? "." : `, 99th percentile ${fmtMs(slow.p99Ms)}.`) +
    (busiest ? ` Busiest: ${busiest.name} at ${pct(busiest.utilization ?? 0)}.` : "");
  // the polite sentence changes every 3 seconds, not with every tick
  const [spoken, setSpoken] = useState(sentence);
  useEffect(() => {
    const t = setInterval(() => setSpoken(sentence), 3000);
    return () => clearInterval(t);
  }, [sentence]);
  const [rate, setRate] = useState("");
  // hello is sent once; every tick carries the rate and the overload now in force
  const now = ticks.at(-1)?.status;
  const shownRate = rate !== "" ? rate : String(now?.rate ?? hello?.rate ?? "");
  const overloaded = now?.overloaded ?? hello?.overloaded ?? false;
  const send = async (n: number) => {
    try {
      await api.liveRate(n);
    } catch (e) {
      notify.warn(e instanceof Error ? e.message : "The rate was refused", "live-rate");
    }
  };
  const commitRate = async () => {
    const n = Number(rate);
    setRate("");
    if (rate === "" || !Number.isFinite(n)) return;
    await send(n);
  };
  // the slider position while it is being moved; it commits on release, never on every move
  const [dragging, setDragging] = useState<number | null>(null);
  const current = Number(shownRate);
  const position = dragging ?? sliderFromRps(Number.isFinite(current) ? current : 0);
  const commitSlider = (v: number) => {
    setDragging(null);
    if (v !== sliderFromRps(Number.isFinite(current) ? current : 0)) void send(rpsFromSlider(v));
  };
  const hasDoc = useDraft((s) => s.view?.doc != null);
  const paused = status === "paused" || status === "hidden-paused";
  return (
    <section
      aria-label="Live figures"
      className="flex flex-wrap items-end gap-x-5 gap-y-1 px-3 py-2 text-sm"
    >
      <dl className="m-0 flex flex-wrap items-end gap-x-5 gap-y-1">
        <div className="flex flex-col gap-0.5">
          <dt className="text-xs text-muted">Arriving</dt>
          <dd className="m-0 font-mono tabular-nums">{twoSignificant(fig.reqPerS)} / s</dd>
        </div>
        <div className="flex flex-col gap-0.5">
          <dt className="text-xs text-muted">Dropped this second</dt>
          <dd
            className={`m-0 font-mono tabular-nums ${dropping ? "font-semibold text-negative" : ""}`}
          >
            {twoSignificant(fig.droppedPerS)}
          </dd>
        </div>
        <div className="flex flex-col gap-0.5">
          <dt className="text-xs text-muted">Goodput</dt>
          <dd className="m-0 font-mono tabular-nums">
            {twoSignificant(Math.max(0, fig.reqPerS - fig.droppedPerS))} / s
          </dd>
        </div>
        <div className="flex flex-col gap-0.5">
          <dt className="text-xs text-muted">Failed in window</dt>
          <dd className="m-0 font-mono tabular-nums">{failedTotal.toLocaleString("en-US")}</dd>
        </div>
        <div className="flex flex-col gap-0.5">
          <dt className="text-xs text-muted">p50 / p99</dt>
          <dd className="m-0 font-mono tabular-nums" title={tooSlow ?? undefined}>
            {slow.p50Ms === null ? "–" : fmtMs(slow.p50Ms)} /{" "}
            {slow.p99Ms === null ? "–" : fmtMs(slow.p99Ms)}
            {tooSlow !== null ? (
              <span className="font-sans text-xs text-warn"> · too slow for a client</span>
            ) : null}
          </dd>
        </div>
        <div className="flex flex-col gap-0.5">
          <dt className="text-xs text-muted">Breaks first</dt>
          <dd
            className="m-0 font-mono tabular-nums"
            title="Estimated from how busy it is now (U = X x S): it holds while the mix and settings stay as they are"
          >
            {firstLimit
              ? `${firstLimit.name} at about ${twoSignificant(firstLimit.saturates_at_req_s ?? 0)} / s`
              : "–"}
          </dd>
        </div>
        <div className="flex flex-col gap-0.5">
          <dt className="text-xs text-muted">Busiest</dt>
          <dd className="m-0 font-mono tabular-nums">
            {busiest
              ? `${busiest.name} ${pct(busiest.utilization ?? 0)}${queue > 0 ? `, ${queue} waiting` : ""}`
              : "–"}
          </dd>
        </div>
      </dl>
      <div className="ml-auto flex items-center gap-2">
        <span className="text-xs text-muted">{STATE_WORDS[status]}</span>
        {throttle ? (
          <>
            <label className="flex items-center gap-1 text-xs text-muted">
              Offered
              <input
                name="live-rate"
                inputMode="decimal"
                value={shownRate}
                onChange={(e) => setRate(e.currentTarget.value)}
                onBlur={() => void commitRate()}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                className="h-7 w-16 rounded border border-control bg-sheet px-1 text-right font-mono text-text"
              />
              / s
            </label>
            <input
              name="live-throttle"
              type="range"
              aria-label="Offered load slider"
              aria-valuetext={`${rpsFromSlider(position)} requests per second`}
              min={0}
              max={SLIDER_MAX}
              step={1}
              value={position}
              onChange={(e) => setDragging(Number(e.currentTarget.value))}
              onPointerUp={(e) => commitSlider(Number(e.currentTarget.value))}
              onKeyUp={(e) => SLIDER_KEYS.has(e.key) && commitSlider(Number(e.currentTarget.value))}
              className="w-40 accent-accent"
            />
          </>
        ) : null}
        <button
          type="button"
          disabled={!hasDoc}
          onClick={openTrafficBuilder}
          title="Build the traffic: endpoints, payload sizes and which services they reach"
          className="flex h-8 items-center gap-1.5 rounded-lg border border-control bg-sheet px-2 text-xs font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50"
        >
          <Workflow size={14} aria-hidden />
          Build traffic
        </button>
        <button
          type="button"
          disabled={status === "off" || status === "connecting"}
          onClick={() => void (paused ? resumeLive() : pauseLive())}
          className="flex h-8 items-center gap-1.5 rounded-lg border border-control bg-sheet px-3 font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50"
        >
          {paused ? <Play size={14} aria-hidden /> : <Pause size={14} aria-hidden />}
          {paused ? "Resume" : "Pause"}
        </button>
      </div>
      {hello?.note ? <p className="m-0 w-full text-xs text-warn">{hello.note}</p> : null}
      {overloaded ? (
        <p className="m-0 w-full text-xs text-warn">
          The system cannot keep up: new requests stop being added until it catches up.
        </p>
      ) : null}
      <output aria-live="polite" className="sr-only">
        {spoken}
      </output>
    </section>
  );
}
