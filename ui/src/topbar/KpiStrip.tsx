// The key figures as type, not tiles (spec 3): what breaks first (the lead, largest), the offered
// load (a log slider and a typed box, the control that moves it), then the system p99, goodput,
// errors and dropped. Every figure comes from the newest run through the same selectors the cards
// use. A slider commits on release, never on every move; a refused typed value puts the real one back.

import { Workflow } from "lucide-react";
import { openTrafficBuilder } from "../traffic/store";
import { HelpPopover } from "../help/HelpPopover";
import type { HelpId } from "../help/copy";
import { useState } from "react";
import { rpsFromSlider, SLIDER_MAX, sliderFromRps } from "../canvas/logScale";
import { breaksFirst, fmtMs, twoSignificant } from "../canvas/metrics";
import { LIMITS, setRate } from "../panel/fields";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";

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

const nf = (n: number, digits = 0) =>
  n.toLocaleString("en-US", { maximumFractionDigits: digits, minimumFractionDigits: digits });

function Cell({
  label,
  help,
  divider = false,
  grow = false,
  title,
  className = "",
  children,
}: {
  label: string;
  /** A rule down its left edge: the start of a new group of figures. */
  divider?: boolean;
  /** Takes the room left over between the figures. */
  grow?: boolean;
  /** The whole figure as a tooltip, for one that has been made smaller or cut to fit. */
  title?: string;
  help?: HelpId;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div
      data-help-host
      className={`flex flex-col gap-0.5 whitespace-nowrap ${grow ? "min-w-0 flex-1" : "shrink-0"} ${divider ? "border-l border-border pl-3" : ""}`}
    >
      <dt className="flex items-center gap-0.5 text-xs text-muted">
        {label}
        {help ? <HelpPopover id={help} label={label} /> : null}
      </dt>
      <dd title={title} className={`m-0 tabular-nums ${className}`}>
        {children}
      </dd>
    </div>
  );
}

export function KpiStrip() {
  const view = useDraft((s) => s.view);
  const patch = useDraft((s) => s.patch);
  const latest = useRun((s) => s.latest);
  const dimmed = useRun((s) => s.dimmed);
  /** The slider position while it is being moved, before it is committed. */
  const [dragging, setDragging] = useState<number | null>(null);

  const traffic = view?.doc?.traffic as Record<string, { rps?: unknown }> | undefined;
  const profile = traffic ? Object.keys(traffic)[0] : undefined;
  const rps = profile !== undefined ? traffic?.[profile]?.rps : undefined;
  const rate = typeof rps === "number" ? Math.round(rps) : null;
  const disabled = profile === undefined || rate === null;
  const position = dragging ?? (rate === null ? 0 : sliderFromRps(rate));
  const shown = dragging !== null ? rpsFromSlider(dragging) : rate;

  const s = latest?.summary ?? undefined;
  const hasRequests = s !== undefined && s.requests > 0 && Number.isFinite(s.latency.p99_ms);
  const goodput = s && s.window_s > 0 ? (s.requests - s.failed) / s.window_s : undefined;
  const dropped = s?.nodes.reduce((n, x) => n + x.limited + x.overflowed, 0);
  const breaks = breaksFirst(s);

  const write = (n: number) => {
    if (profile === undefined) return;
    void patch([setRate(profile, n)]);
  };
  const commitSlider = (v: number) => {
    setDragging(null);
    if (v === (rate === null ? 0 : sliderFromRps(rate))) return; // a Tab onto it, or no move
    write(rpsFromSlider(v));
  };
  const commitTyped = (el: HTMLInputElement) => {
    const n = Math.round(Number(el.value));
    // zero, a negative or text is refused; a huge number is held to the top of the range (R1.100)
    if (el.value.trim() === "" || !Number.isFinite(n) || n < 1) {
      el.value = String(rate ?? "");
      return;
    }
    const clamped = Math.min(n, LIMITS.rate.max);
    el.value = String(clamped);
    if (clamped !== rate) write(clamped);
  };

  // the lead figure; a long name or a big rate makes it smaller (and the whole of it is the tooltip)
  const lead =
    breaks === null
      ? s === undefined
        ? "Run to find out"
        : "nothing saturates"
      : `${breaks.node} at ${twoSignificant(breaks.atReqS)} req/s`;

  const marker =
    breaks !== null && breaks.atReqS >= 10 && breaks.atReqS <= 15000
      ? (sliderFromRps(breaks.atReqS) / SLIDER_MAX) * 100
      : null;

  return (
    <section
      aria-label="Key figures"
      data-tour="load"
      className="flex w-full items-end px-3 py-2.5"
    >
      <dl
        className={`m-0 flex w-full flex-nowrap items-end justify-between gap-x-3 ${dimmed ? "opacity-60" : ""}`}
      >
        <Cell
          label="Breaks first"
          help="saturation"
          title={lead}
          className={`min-w-40 max-w-72 truncate font-semibold ${
            lead.length > 26 ? "text-sm" : lead.length > 22 ? "text-base" : "text-xl"
          } ${
            breaks === null ? "text-muted" : breaks.saturatedNow ? "text-negative" : "text-text"
          }`}
        >
          {lead}
        </Cell>
        <Cell label="Offered load" divider grow className="text-lg font-semibold">
          <span className="flex w-full items-center gap-2">
            <input
              key={`${shown}`}
              type="number"
              aria-label="Offered load, typed (requests per second)"
              min={1}
              max={LIMITS.rate.max}
              disabled={disabled}
              defaultValue={shown ?? ""}
              onBlur={(e) => commitTyped(e.currentTarget)}
              onKeyDown={(e) => e.key === "Enter" && commitTyped(e.currentTarget)}
              className="h-8 w-20 rounded-md border border-control bg-sheet px-2 text-right text-lg"
            />
            <span className="text-base text-muted">/s</span>
            <span className="relative flex min-w-16 max-w-96 flex-1 items-center">
              <input
                type="range"
                aria-label="Offered load"
                aria-valuetext={
                  shown === null ? "no traffic profile" : `${nf(shown)} requests per second`
                }
                min={0}
                max={SLIDER_MAX}
                step={1}
                disabled={disabled}
                value={position}
                onChange={(e) => setDragging(Number(e.currentTarget.value))}
                onPointerUp={(e) => commitSlider(Number(e.currentTarget.value))}
                onKeyUp={(e) => {
                  if (SLIDER_KEYS.has(e.key)) commitSlider(Number(e.currentTarget.value));
                }}
                className="w-full accent-accent"
              />
              {marker !== null && breaks !== null ? (
                <span
                  aria-hidden="true"
                  title={`Breaks first at ${twoSignificant(breaks.atReqS)} req/s`}
                  className="pointer-events-none absolute inset-y-0 w-0.5 -translate-x-1/2 bg-negative"
                  style={{ left: `${marker}%` }}
                />
              ) : null}
            </span>
            <button
              type="button"
              disabled={view?.doc == null}
              onClick={openTrafficBuilder}
              title="Build the traffic: endpoints, payload sizes and which services they reach"
              className="flex h-8 items-center gap-1.5 rounded-lg border border-control bg-sheet px-2 text-xs font-medium text-text hover:bg-raised disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
            >
              <Workflow size={14} aria-hidden />
              <span className="max-[1699px]:sr-only">Build traffic</span>
            </button>
          </span>
        </Cell>
        <Cell
          label="System p99"
          divider
          help="p99"
          className={`text-sm ${hasRequests ? "" : "text-muted"}`}
        >
          {hasRequests && s ? fmtMs(s.latency.p99_ms, 1) : "–"}
        </Cell>
        <Cell
          label="Goodput"
          help="goodput"
          className={`text-sm ${goodput === undefined ? "text-muted" : ""}`}
        >
          {goodput === undefined ? "–" : `${nf(goodput)}/s`}
        </Cell>
        <Cell label="Errors" className={`text-sm ${s ? "" : "text-muted"}`}>
          {s ? nf(s.failed) : "–"}
        </Cell>
        <Cell label="Dropped" className={`text-sm ${dropped === undefined ? "text-muted" : ""}`}>
          {dropped === undefined ? "–" : nf(dropped)}
        </Cell>
      </dl>
    </section>
  );
}
