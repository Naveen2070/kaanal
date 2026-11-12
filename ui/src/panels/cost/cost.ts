// The cost estimate as arithmetic (RULES 121): copies x price an hour, added up. Money is kept in
// whole micro-USD (1,000,000 = 1 dollar) until it is shown, so sums are exact. Pure; the card
// (CostCard.tsx) fetches the prices. An estimate for a what-if, never a quote: on-demand Linux list
// prices of one region, instances only (no network, storage or managed-service fees).

import { replicasOf } from "../../canvas/metrics";
import type { Doc } from "../../data/api/types";
import type { StoragePrice } from "../../data/api/prices";
import { parseBytes } from "../details/units";
import { customInstancesOf } from "../details/fields";
import { priceListKey } from "../details/hosting";

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

/** Where a component runs (`hosting:`, RULES 122); `self` when the file says nothing. */
export type Hosting = "self" | "managed" | "serverless" | "external";

type Cfg = Readonly<Record<string, unknown>>;

export function hostingOf(config: Cfg): Hosting {
  const h = config.hosting;
  return h === "managed" || h === "serverless" || h === "external" ? h : "self";
}

/** A managed database's engine (`engine:`), PostgreSQL when the file says nothing. */
export function engineOf(config: Cfg): string {
  return typeof config.engine === "string" && config.engine !== "" ? config.engine : "postgresql";
}

/** A price list a node reads: its table key (`aws`, or `aws-rds-mysql` for a managed class) and the provider whose region it is priced in. */
export interface PriceKey {
  key: string;
  provider: string;
}

/** The list a node's instance is priced from; null when it names none or is your own instance. */
export function priceKeyOf(config: Cfg, mine: Record<string, unknown>): PriceKey | null {
  const i = typeof config.instance === "string" ? config.instance.trim() : "";
  if (i === "" || i in mine) return null;
  const provider = i.split("/")[0]?.toLowerCase();
  if (!provider) return null;
  // a managed class is priced in its own list (per engine for a database), in the provider's region
  const key = priceListKey(config, provider);
  return key === null ? null : { key, provider };
}

/** The price lists a document's nodes read, sorted. */
export function priceKeysUsed(doc: Doc | null): PriceKey[] {
  const mine = customInstancesOf(doc);
  const out = new Map<string, PriceKey>();
  for (const n of Object.values(doc?.nodes ?? {})) {
    const k = priceKeyOf(n, mine);
    if (k) out.set(k.key, k);
  }
  return [...out.values()].sort((x, y) => (x.key < y.key ? -1 : 1));
}

/** The cloud an object store is priced as: `gcs` Google, `blob` Azure, `s3` or no flavor AWS. */
export function storageProvider(config: Cfg): string {
  return config.flavor === "gcs" ? "gcp" : config.flavor === "blob" ? "azure" : "aws";
}

/** Hours in a month for turning a GB-month into an hour (a year of 8,760 hours over 12). */
export const HOURS_PER_MONTH = 730;

/** The providers a document's nodes name in `instance:`, and the cloud of each storage node (a custom instance has none). */
export function providersUsed(doc: Doc | null): string[] {
  const out = new Set(priceKeysUsed(doc).map((k) => k.provider));
  for (const n of Object.values(doc?.nodes ?? {}))
    if (n.type === "storage") out.add(storageProvider(n));
  return [...out].sort();
}

/** What the last run saw: visits a second per node name, for pricing an object store's requests. */
export type VisitRates = Readonly<Record<string, number>>;

/**
 * An object store's price an hour in micro-USD (RULES 123), S3-style: the data kept (`stored`)
 * over a month, plus its requests at the last run's visit rate, a `write_share` of them at the
 * write price and the rest at the read price. Copies do not multiply it: one store, one bill.
 */
function storageCost(
  name: string,
  config: Cfg,
  region: Record<string, string>,
  prices: Readonly<Record<string, StoragePrice>> | undefined,
  visits: VisitRates | undefined,
): { micro: number | null; why: string | null } {
  const provider = storageProvider(config);
  if (prices === undefined) return { micro: null, why: "prices are loading" };
  const r = region[provider] ?? "";
  const p = prices[`${provider}/${r}`] ?? prices[`${provider}/*`];
  if (!p) return { micro: null, why: `no object-store price in ${r || "this region"}` };
  const bytes = config.stored === undefined ? null : parseBytes(config.stored);
  const gb = (bytes ?? 0) / 1024 ** 3;
  const rate = visits?.[name] ?? 0;
  if (bytes === null && rate === 0) {
    return { micro: null, why: "set stored: (the data kept); requests are priced after a run" };
  }
  const w = typeof config.write_share === "number" ? config.write_share : 0;
  const perReq = ((1 - w) * p.readPer1k + w * p.writePer1k) / 1000;
  return {
    micro: Math.round((gb * p.gbMonth) / HOURS_PER_MONTH + rate * 3600 * perReq),
    why: null,
  };
}

/** Why a node has no price when its hosting leaves the instance out, or null when it can have one. */
function hostingWhy(config: Cfg): string | null {
  switch (hostingOf(config)) {
    case "serverless":
      return "serverless: priced per use, not estimated";
    case "external":
      return "run by someone else: not estimated";
    case "managed":
      return config.type === "database" || config.type === "cache"
        ? null
        : "managed: priced by use, not estimated";
    default:
      return null;
  }
}

/** Each node's price an hour, in node order. `tables` is keyed by price list (`PriceKey.key`). */
export function nodeCosts(
  doc: Doc | null,
  tables: PriceTables,
  region: Record<string, string>,
  storage?: { prices?: Readonly<Record<string, StoragePrice>>; visits?: VisitRates },
): NodeCost[] {
  const mine = customInstancesOf(doc);
  return Object.entries(doc?.nodes ?? {}).map(([name, config]) => {
    const instance =
      typeof config.instance === "string" && config.instance.trim() !== ""
        ? config.instance.trim()
        : null;
    if (config.type === "storage") {
      const s = storageCost(name, config, region, storage?.prices, storage?.visits);
      return {
        name,
        instance: null,
        copies: 1,
        unitMicro: s.micro,
        totalMicro: s.micro,
        why: s.why,
      };
    }
    const copies = replicasOf(config);
    let unitMicro: number | null = null;
    let why: string | null = hostingWhy(config);
    if (why !== null) {
      // the hosting has no machine to price
    } else if (instance === null) {
      why =
        hostingOf(config) === "managed"
          ? `pick a managed ${config.type === "cache" ? "cache" : "database"} class`
          : "no instance type";
    } else if (instance in mine) {
      const c = mine[instance]?.cost ?? null;
      if (c === null) why = "your instance has no cost_per_hour";
      else unitMicro = Math.round(c * 1e6);
    } else {
      const k = priceKeyOf(config, mine);
      const table = k ? tables[k.key] : undefined;
      if (table === undefined) why = "prices are loading";
      else {
        const p = table.get(instance.toLowerCase());
        if (p === undefined) why = `no price in ${region[k?.provider ?? ""] ?? "this region"}`;
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
