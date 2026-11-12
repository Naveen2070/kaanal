// The cloud region each provider is priced and listed in (RULES 121): one choice for the whole page,
// shared by the instance picker (it lists only the types the region has), the cost estimate and the
// region menu at the top right. It is a setting of this browser, not of the file: the region does
// not change what a type does, only what it costs and whether it is offered there. The default of
// each provider is the one the instance table carries (AWS us-east-1, Azure us-east, GCP
// us-central1).

import { useEffect, useState } from "react";
import { create } from "zustand";
import { loadPriceRegions, loadPrices } from "../api/prices";

const KEY = "kaanal.regions";

function stored(): Record<string, string> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Record<string, unknown>;
    return Object.fromEntries(Object.entries(v).filter(([, r]) => typeof r === "string")) as Record<
      string,
      string
    >;
  } catch {
    return {};
  }
}

export interface RegionState {
  /** The region this browser picked for each provider; a provider with none uses its default. */
  picked: Record<string, string>;
  defaults: Record<string, string>;
  regions: Record<string, string[]>;
  /** The lists have arrived. */
  ready: boolean;
  /** Fetches the region lists (once; a failure is tried again on the next call). */
  load: () => void;
  pick: (provider: string, region: string) => void;
}

export const useRegion = create<RegionState>((set, get) => ({
  picked: stored(),
  defaults: {},
  regions: {},
  ready: false,
  load: () => {
    if (get().ready) return;
    loadPriceRegions().then(
      (r) => set({ defaults: r.defaults, regions: r.regions, ready: true }),
      () => undefined,
    );
  },
  pick: (provider, region) => {
    const picked = { ...get().picked, [provider]: region };
    set({ picked });
    try {
      localStorage.setItem(KEY, JSON.stringify(picked));
    } catch {
      // storage blocked: the choice holds for this page only
    }
  },
}));

/** The region in force for `provider`: the one picked, else its default; undefined until known. */
export function regionOf(s: RegionState, provider: string): string | undefined {
  return s.picked[provider] ?? s.defaults[provider];
}

/**
 * The types of `provider` that have a price in its region, which is how the table says a type is
 * offered there (name to micro-USD an hour); `undefined` while it loads, and `null` when it could
 * not be loaded (the picker then lists every type, as before).
 */
export function useRegionTable(
  provider: string | null,
): ReadonlyMap<string, number> | undefined | null {
  const region = useRegion((s) => (provider === null ? undefined : regionOf(s, provider)));
  const load = useRegion((s) => s.load);
  const [table, setTable] = useState<{
    key: string;
    t: ReadonlyMap<string, number> | null;
  } | null>(null);
  useEffect(() => {
    if (provider !== null) load();
  }, [provider, load]);
  useEffect(() => {
    if (provider === null || region === undefined) return;
    let gone = false;
    const key = `${provider}/${region}`;
    loadPrices(provider, region).then(
      (t) => !gone && setTable({ key, t }),
      () => !gone && setTable({ key, t: null }),
    );
    return () => {
      gone = true;
    };
  }, [provider, region]);
  if (provider === null || region === undefined) return undefined;
  return table?.key === `${provider}/${region}` ? table.t : undefined;
}
