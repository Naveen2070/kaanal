// A small card under the node picker (RULES 121): the estimated cost of the system as it is drawn.
// Collapsed it shows one figure, dollars a day; the button opens a panel beside it with the days
// to estimate for, the region of each cloud in use, the price of every node (copies x price an
// hour), the totals and the cost per 1,000 requests at the offered rate. It is an estimate and
// says so: on-demand Linux list prices of one region, instances only.

import { CircleDollarSign, X } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { loadPrices, loadStoragePrices, type StoragePrice } from "../../data/api/prices";
import { useRun } from "../../data/store/run";
import { useDraft } from "../../data/store/draft";
import { useLive } from "../../data/store/live";
import { nodeVisitRates } from "../../data/store/liveMath";
import { isLive, useMode } from "../../data/store/mode";
import { regionOf, useRegion } from "../../data/store/region";
import { useCompactPalette } from "../../shell/layout/useLayout";
import { activeProfile } from "../traffic/model";
import {
  daysFrom,
  dollarsFor,
  MAX_DAYS,
  nodeCosts,
  perThousandRequests,
  priceKeysUsed,
  providersUsed,
  totals,
  usd,
  type PriceTables,
} from "./cost";

const PROVIDER_LABEL: Record<string, string> = { aws: "AWS", azure: "Azure", gcp: "Google Cloud" };
const KEY = "kaanal.cost";

/** The days this browser remembers for the card. */
function storedDays(): number {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as { days?: unknown };
    return daysFrom(String(v.days), 30);
  } catch {
    return 30;
  }
}

function rememberDays(days: number): void {
  try {
    localStorage.setItem(KEY, JSON.stringify({ days }));
  } catch {
    // a private window or blocked storage: the card still works, it just forgets
  }
}

export function CostCard() {
  const doc = useDraft((s) => s.view?.doc ?? null);
  const compact = useCompactPalette();
  const mode = useMode((s) => s.mode);
  const liveRate = useLive((s) => s.ticks.at(-1)?.status.rate ?? s.hello?.rate ?? 0);
  const [open, setOpen] = useState(false);
  const [days, setDays] = useState(storedDays);
  const [daysText, setDaysText] = useState(() => String(storedDays()));
  const regionState = useRegion();
  const [tables, setTables] = useState<Record<string, ReadonlyMap<string, number> | undefined>>({});
  const [problem, setProblem] = useState<string | null>(null);
  // object stores are priced from one list of every region, and their requests from the last run
  const [storagePrices, setStoragePrices] = useState<Record<string, StoragePrice> | undefined>();
  const runNodes = useRun((s) => s.latest?.summary?.nodes);
  const hasStorage = Object.values(doc?.nodes ?? {}).some((n) => n.type === "storage");
  useEffect(() => {
    if (!hasStorage) return;
    let gone = false;
    loadStoragePrices().then(
      (p) => !gone && setStoragePrices(p),
      () => !gone && setProblem("No object-store prices."),
    );
    return () => {
      gone = true;
    };
  }, [hasStorage]);
  // a store's requests are priced at what the live run is doing now (Live and Admin), else at the
  // last batch run's visits
  const liveTicks = useLive((s) => s.ticks);
  const liveNames = useLive((s) => s.hello?.nodes);
  const visits = useMemo(() => {
    const live = isLive(mode) ? nodeVisitRates(liveTicks, liveNames ?? []) : {};
    if (Object.keys(live).length > 0) return live;
    return Object.fromEntries((runNodes ?? []).map((n) => [n.name, n.visits_per_s]));
  }, [mode, liveTicks, liveNames, runNodes]);
  const button = useRef<HTMLButtonElement>(null);
  const [at, setAt] = useState<{ left: number; bottom: number } | null>(null);
  const id = useId();

  const used = providersUsed(doc);
  const usedKey = used.join(",");
  // fetched when a cloud instance is in the draft, not for every page that has no instance
  const loadRegions = regionState.load;
  useEffect(() => {
    if (usedKey !== "") loadRegions();
  }, [usedKey, loadRegions]);

  const region = (p: string): string | undefined => regionOf(regionState, p);
  // a price list is read in the region of its provider: a managed class (aws-rds-mysql) in the AWS one
  const keys = priceKeysUsed(doc);
  const regionKey = keys.map((k) => `${k.key}/${region(k.provider) ?? ""}`).join(",");
  useEffect(() => {
    let gone = false;
    for (const x of regionKey === "" ? [] : regionKey.split(",")) {
      const [p, r] = x.split("/");
      if (!p || !r) continue;
      loadPrices(p, r).then(
        (t) => !gone && setTables((s) => ({ ...s, [p]: t })),
        () => !gone && setProblem(`No prices for ${p} in ${r}.`),
      );
    }
    return () => {
      gone = true;
    };
    // the region of each provider in use is the key: a change of region fetches again
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [regionKey]);

  const regionNow = Object.fromEntries(used.map((p) => [p, region(p) ?? ""]));
  const costs = useMemo(
    () => nodeCosts(doc, tables as PriceTables, regionNow, { prices: storagePrices, visits }),
    // regionNow is derived from regionKey
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [doc, tables, regionKey, storagePrices, visits],
  );
  const sum = totals(costs);
  const profile = activeProfile(doc);
  const traffic = doc?.traffic as Record<string, { rps?: unknown }> | undefined;
  const profileRps = typeof traffic?.[profile]?.rps === "number" ? traffic[profile].rps : 0;
  const rate = isLive(mode) && liveRate > 0 ? liveRate : profileRps;
  const per1k = perThousandRequests(sum.perHourMicro, rate);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        setOpen(false);
        button.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  if (!doc?.nodes) return null;
  const toggle = () => {
    const r = button.current?.getBoundingClientRect();
    // anchored to the card's bottom edge so the panel grows upward and stays on the screen
    if (r) setAt({ left: r.right + 8, bottom: Math.max(8, window.innerHeight - r.bottom) });
    setOpen((o) => !o);
  };
  const setDaysTo = (text: string) => {
    setDaysText(text);
    const n = daysFrom(text, days);
    setDays(n);
    rememberDays(n);
  };
  const perDay = dollarsFor(sum.perHourMicro, 1);

  return (
    <div className={`pointer-events-auto ${compact ? "w-14" : "w-76"}`}>
      <button
        ref={button}
        type="button"
        aria-expanded={open}
        aria-controls={`${id}-panel`}
        aria-label={`Estimated cost: ${sum.priced > 0 ? `${usd(perDay)} a day` : "no prices yet"}. ${open ? "Close" : "Open"} the cost estimate`}
        onClick={toggle}
        className={`flex w-full items-center gap-2 rounded-2xl border border-border bg-sheet p-2 text-left shadow-float hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent ${compact ? "flex-col justify-center" : ""}`}
      >
        <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-well">
          <CircleDollarSign size={16} aria-hidden className="text-muted" />
        </span>
        <span
          className={compact ? "text-center text-[10px] leading-tight" : "min-w-0 flex-1 text-xs"}
        >
          <span className="block font-semibold tabular-nums">
            {sum.priced > 0 ? `${usd(perDay)}${compact ? "" : " a day"}` : "No prices yet"}
          </span>
          {compact ? (
            <span className="block text-muted">/day</span>
          ) : (
            <span className="block truncate text-muted">estimate · {sum.priced} priced</span>
          )}
        </span>
      </button>
      {open && at ? (
        <section
          id={`${id}-panel`}
          aria-label="Cost estimate"
          style={{ left: at.left, bottom: at.bottom }}
          className="fixed z-30 max-h-[calc(100vh-1rem)] w-[min(26rem,calc(100vw-5rem))] overflow-y-auto rounded-2xl border border-border bg-sheet p-3 text-sm shadow-float"
        >
          <div className="flex items-start justify-between gap-2">
            <h2 className="m-0 text-sm font-semibold">Cost estimate</h2>
            <button
              type="button"
              aria-label="Close the cost estimate"
              onClick={() => {
                setOpen(false);
                button.current?.focus();
              }}
              className="flex size-6 items-center justify-center rounded-md hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
            >
              <X size={14} aria-hidden />
            </button>
          </div>
          <p className="m-0 mt-1 text-xs text-warn">
            An estimate, not a quote: on-demand Linux list prices of one region, instances only (no
            network, storage or managed-service fees). Burstable and spot pricing are not modelled.
          </p>
          <div className="mt-2 flex flex-wrap items-end gap-x-3 gap-y-2">
            <label className="flex flex-col gap-0.5 text-xs text-muted">
              Days to estimate
              <input
                name="cost-days"
                type="number"
                inputMode="numeric"
                min={1}
                max={MAX_DAYS}
                step={1}
                value={daysText}
                onChange={(e) => setDaysTo(e.currentTarget.value)}
                onBlur={() => setDaysText(String(days))}
                className="h-7 w-20 rounded border border-control bg-sheet px-1 text-right font-mono tabular-nums text-text"
              />
            </label>
            {used.map((p) => (
              <label key={p} className="flex flex-col gap-0.5 text-xs text-muted">
                {PROVIDER_LABEL[p] ?? p} region
                <select
                  name={`cost-region-${p}`}
                  value={region(p) ?? ""}
                  disabled={!regionState.ready}
                  onChange={(e) => regionState.pick(p, e.currentTarget.value)}
                  className="h-7 max-w-44 rounded border border-control bg-sheet px-1 text-text"
                >
                  {(regionState.regions[p] ?? []).map((r) => (
                    <option key={r} value={r}>
                      {r}
                    </option>
                  ))}
                </select>
              </label>
            ))}
          </div>
          {problem ? <p className="mt-2 text-xs text-warn">{problem}</p> : null}
          <table className="mt-3 w-full text-xs">
            <thead>
              <tr className="text-left text-muted">
                <th className="font-normal">Node</th>
                <th className="font-normal">Copies</th>
                <th className="text-right font-normal">Each / h</th>
                <th className="text-right font-normal">Total / day</th>
              </tr>
            </thead>
            <tbody>
              {costs.map((c) => (
                <tr key={c.name} className="border-t border-border align-top">
                  <td className="max-w-32 py-1 pr-1">
                    <span className="block truncate font-medium" title={c.name}>
                      {c.name}
                    </span>
                    <span
                      className="block truncate font-mono text-[11px] text-muted"
                      title={c.instance ?? undefined}
                    >
                      {c.instance ?? "no instance type"}
                    </span>
                  </td>
                  <td className="py-1 tabular-nums">{c.copies}</td>
                  {c.unitMicro === null || c.totalMicro === null ? (
                    <td colSpan={2} className="py-1 text-right text-muted">
                      {c.why}
                    </td>
                  ) : (
                    <>
                      <td className="py-1 text-right tabular-nums">{usd(c.unitMicro / 1e6)}</td>
                      <td className="py-1 text-right tabular-nums">
                        {usd(dollarsFor(c.totalMicro, 1))}
                      </td>
                    </>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
          <dl className="m-0 mt-2 grid grid-cols-[1fr_auto] gap-x-3 gap-y-0.5 border-t border-border pt-2 text-xs">
            <dt className="text-muted">An hour</dt>
            <dd className="m-0 text-right font-mono tabular-nums">{usd(sum.perHourMicro / 1e6)}</dd>
            <dt className="text-muted">A day</dt>
            <dd className="m-0 text-right font-mono tabular-nums">{usd(perDay)}</dd>
            <dt className="font-medium">{days === 1 ? "1 day" : `${days} days`}</dt>
            <dd className="m-0 text-right font-mono font-semibold tabular-nums">
              {usd(dollarsFor(sum.perHourMicro, days))}
            </dd>
            <dt className="text-muted">Per 1,000 requests</dt>
            <dd className="m-0 text-right font-mono tabular-nums">
              {per1k === null ? "no load set" : usd(per1k)}
            </dd>
          </dl>
          <p className="m-0 mt-1 text-[11px] text-muted">
            {rate > 0
              ? `Per 1,000 requests at ${rate.toLocaleString("en-US", { maximumFractionDigits: 1 })} requests a second (${isLive(mode) && liveRate > 0 ? "the offered rate now" : "the traffic profile"}).`
              : "Add a traffic profile or offer load to see the cost per 1,000 requests."}
            {sum.unpriced > 0
              ? ` ${sum.unpriced} ${sum.unpriced === 1 ? "node is" : "nodes are"} left out for want of a price.`
              : ""}
          </p>
        </section>
      ) : null}
    </div>
  );
}
