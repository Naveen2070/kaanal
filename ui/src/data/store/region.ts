// The cloud region each provider is priced and listed in (RULES 121): one choice for the whole page,
// shared by the instance picker (it lists only the types the region has), the cost estimate and the
// region menu at the top right. A project can carry it: `region: { aws: eu-west-1 }` in the file wins
// (RULES 125), and choosing a region at the globe writes it there. A file with no `region:` for a
// provider uses what this browser picked, else the provider's default. The region does not change
// what a type does, only what it costs and whether it is offered there. The default of each
// provider is the one the instance table carries (AWS us-east-1, Azure us-east, GCP us-central1).

import { useEffect, useState } from "react";
import { create } from "zustand";
import { loadPriceRegions, loadPrices } from "../api/prices";
import type { Doc, Op } from "../api/types";
import { useDraft } from "./draft";

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
  /** What the file's `region:` says; it wins over `picked`. */
  fromFile: Record<string, string>;
  defaults: Record<string, string>;
  regions: Record<string, string[]>;
  /** The lists have arrived. */
  ready: boolean;
  /** Fetches the region lists (once; a failure is tried again on the next call). */
  load: () => void;
  pick: (provider: string, region: string) => void;
  setFile: (regions: Record<string, string>) => void;
}

export const useRegion = create<RegionState>((set, get) => ({
  picked: stored(),
  fromFile: {},
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
  setFile: (fromFile) => set({ fromFile }),
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

/** The region in force for `provider`: the file's, else the one this browser picked, else its default; undefined until known. */
export function regionOf(s: RegionState, provider: string): string | undefined {
  return s.fromFile[provider] ?? s.picked[provider] ?? s.defaults[provider];
}

/** The `region:` mapping of a document: provider to region, strings only; empty when it has none. */
export function fileRegions(doc: Doc | null | undefined): Record<string, string> {
  const r = doc?.region;
  if (r === null || typeof r !== "object" || Array.isArray(r)) return {};
  return Object.fromEntries(Object.entries(r).filter(([, v]) => typeof v === "string")) as Record<
    string,
    string
  >;
}

/** The patch that writes `region: { provider: region }` into the file, creating the mapping when it is missing. */
export function regionOp(doc: Doc | null | undefined, provider: string, region: string): Op {
  const file = fileRegions(doc);
  if (doc?.region === undefined)
    return { op: "insert", path: ["region"], value: { [provider]: region } };
  return { op: provider in file ? "set" : "insert", path: ["region", provider], value: region };
}

/** Keeps the store's `fromFile` equal to the draft's `region:`; mount it once (the top bar does). */
export function useFileRegion(): void {
  const doc = useDraft((s) => s.view?.doc);
  const key = JSON.stringify(fileRegions(doc));
  const setFile = useRegion((s) => s.setFile);
  useEffect(() => setFile(JSON.parse(key) as Record<string, string>), [key, setFile]);
}

/**
 * The types of `provider` that have a price in its region, which is how the table says a type is
 * offered there (name to micro-USD an hour); `undefined` while it loads, and `null` when it could
 * not be loaded (the picker then lists every type, as before). `list` is the price list read when
 * it is not the provider's own: a managed database class is priced in `aws-rds-<engine>`, in the
 * region of AWS.
 */
export function useRegionTable(
  provider: string | null,
  list: string | null = provider,
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
    const key = `${list ?? provider}/${region}`;
    loadPrices(list ?? provider, region).then(
      (t) => !gone && setTable({ key, t }),
      () => !gone && setTable({ key, t: null }),
    );
    return () => {
      gone = true;
    };
  }, [provider, list, region]);
  if (provider === null || region === undefined) return undefined;
  return table?.key === `${list ?? provider}/${region}` ? table.t : undefined;
}
