// The cost estimate as arithmetic (RULES 121): copies x price an hour, added up. Money is kept in
// whole micro-USD (1,000,000 = 1 dollar) until it is shown, so sums are exact. Pure; the card
// (CostCard.tsx) fetches the prices. An estimate for a what-if, never a quote: on-demand Linux list
// prices of one region, instances only (no network, storage or managed-service fees).

import { replicasOf } from "../../canvas/metrics";
import type { Doc } from "../../data/api/types";
import { customInstancesOf } from "../details/fields";

export const HOURS_PER_DAY = 24;
/** A day is shown as `days`; at most about ten years. */
export const MAX_DAYS = 3650;

/** The prices of each provider's picked region, `provider/type` to micro-USD; a missing provider is still loading. */
export type PriceTables = Readonly<Record<string, ReadonlyMap<string, number> | undefined>>;

export interface NodeCost {
  name: string;
  /** The instance type the node names, null for none. */
  instance: string | null;
  copies: number;
  /** One copy an hour, micro-USD; null when there is no price (see `why`). */
  unitMicro: number | null;
  /** All copies an hour, micro-USD. */
  totalMicro: number | null;
  /** Why there is no price, in words. */
  why: string | null;
}

/** The providers a document's nodes name in `instance:` (a custom instance has none). */
export function providersUsed(doc: Doc | null): string[] {
  const mine = customInstancesOf(doc);
  const out = new Set<string>();
  for (const n of Object.values(doc?.nodes ?? {})) {
    const i = typeof n.instance === "string" ? n.instance.trim() : "";
    if (i === "" || i in mine) continue;
    const p = i.split("/")[0]?.toLowerCase();
    if (p) out.add(p);
  }
  return [...out].sort();
}

/** Each node's price an hour, in node order. */
export function nodeCosts(
  doc: Doc | null,
  tables: PriceTables,
  region: Record<string, string>,
): NodeCost[] {
  const mine = customInstancesOf(doc);
  return Object.entries(doc?.nodes ?? {}).map(([name, config]) => {
    const instance =
      typeof config.instance === "string" && config.instance.trim() !== ""
        ? config.instance.trim()
        : null;
    const copies = replicasOf(config);
    let unitMicro: number | null = null;
    let why: string | null = null;
    if (instance === null) why = "no instance type";
    else if (instance in mine) {
      const c = mine[instance]?.cost ?? null;
      if (c === null) why = "your instance has no cost_per_hour";
      else unitMicro = Math.round(c * 1e6);
    } else {
      const provider = instance.split("/")[0]?.toLowerCase() ?? "";
      const table = tables[provider];
      if (table === undefined) why = "prices are loading";
      else {
        const p = table.get(instance.toLowerCase());
        if (p === undefined) why = `no price in ${region[provider] ?? "this region"}`;
        else unitMicro = p;
      }
    }
    return {
      name,
      instance,
      copies,
      unitMicro,
      totalMicro: unitMicro === null ? null : unitMicro * copies,
      why,
    };
  });
}

export interface CostTotals {
  /** Everything priced, an hour, micro-USD. */
  perHourMicro: number;
  priced: number;
  unpriced: number;
}

export function totals(costs: readonly NodeCost[]): CostTotals {
  let perHourMicro = 0;
  let priced = 0;
  for (const c of costs) {
    if (c.totalMicro === null) continue;
    perHourMicro += c.totalMicro;
    priced++;
  }
  return { perHourMicro, priced, unpriced: costs.length - priced };
}

/** Dollars for `days` days of `perHourMicro`. */
export const dollarsFor = (perHourMicro: number, days: number): number =>
  (perHourMicro * HOURS_PER_DAY * days) / 1e6;

/** Dollars per 1,000 requests at `reqPerS`; null at no load. */
export function perThousandRequests(perHourMicro: number, reqPerS: number): number | null {
  return reqPerS > 0 ? (perHourMicro / 1e6 / (reqPerS * 3600)) * 1000 : null;
}

/** A dollar amount for a sentence: whole dollars from 100, cents from 1, two figures below. */
export function usd(d: number): string {
  if (d >= 100) return `$${Math.round(d).toLocaleString("en-US")}`;
  if (d >= 1) return `$${d.toFixed(2)}`;
  if (d === 0) return "$0";
  return `$${d.toPrecision(2)}`;
}

/** The days box's text as a number of days, 1 to `MAX_DAYS`; `fallback` when it is not one. */
export function daysFrom(text: string, fallback: number): number {
  const n = Math.floor(Number(text));
  return Number.isFinite(n) && n >= 1 ? Math.min(n, MAX_DAYS) : fallback;
}
