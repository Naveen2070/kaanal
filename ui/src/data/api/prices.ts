// The per-region price lists the cost estimate reads (RULES 121): the regions of each provider, and
// one region's on-demand Linux prices an hour in micro-USD, fetched when asked for and kept. Estimates.

import { api } from "./client";

export interface PriceRegions {
  defaults: Record<string, string>;
  regions: Record<string, string[]>;
}

let regionsOnce: Promise<PriceRegions> | null = null;
const tables = new Map<string, Promise<ReadonlyMap<string, number>>>();

/** The regions of every provider and each one's default; fetched once, a failure is tried again. */
export function loadPriceRegions(): Promise<PriceRegions> {
  regionsOnce ??= api.getPriceRegions().catch((e: unknown) => {
    regionsOnce = null;
    throw e;
  });
  return regionsOnce;
}

/** `provider/type` (lower case) to micro-USD an hour, for one region. */
export function loadPrices(provider: string, region: string): Promise<ReadonlyMap<string, number>> {
  const key = `${provider}/${region}`;
  let t = tables.get(key);
  if (!t) {
    t = api.getPrices(provider, region).then(
      (r) => new Map(r.prices),
      (e: unknown) => {
        tables.delete(key);
        throw e;
      },
    );
    tables.set(key, t);
  }
  return t;
}

/** Forgets everything fetched (tests). */
export function resetPrices(): void {
  regionsOnce = null;
  tables.clear();
}
