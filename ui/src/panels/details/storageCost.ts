// The two inputs of an object store's cost estimate (RULES 123): the data kept (`stored`) and the
// share of calls that are writes (`write_share`). They change no timing, only the price.

import type { NodeDoc, Op } from "../../data/api/types";
import { put } from "./fields.base";
import { parseBytes } from "./units";

/** The bytes kept in the store (`stored: 500GB`); null = none written. */
export function storedOf(config: NodeDoc): number | null {
  return config.stored === undefined ? null : parseBytes(config.stored);
}

/** The patch for the data kept: a size in bytes writes it, 0 or less removes it. */
export function setStored(name: string, config: NodeDoc, bytes: number): Op[] {
  const path = ["nodes", name, "stored"];
  const present = "stored" in config;
  if (!(bytes > 0)) return present ? [{ op: "remove", path }] : [];
  return [put(path, present, Math.round(bytes))];
}

/** The share of calls that are writes, 0 to 1; null = none written (every call is priced as a read). */
export function writeShareOf(config: NodeDoc): number | null {
  const v = config.write_share;
  return typeof v === "number" && v >= 0 && v <= 1 ? v : null;
}

/** The patch for the write share: 0 removes it (the default), a share from 0 to 1 writes it. */
export function setWriteShare(name: string, config: NodeDoc, share: number): Op[] {
  const path = ["nodes", name, "write_share"];
  const present = "write_share" in config;
  if (!(share > 0)) return present ? [{ op: "remove", path }] : [];
  return [put(path, present, Math.min(1, share))];
}

/** Where the store is (`location:`), for its price instead of the globe's region; "" = none written. */
export const locationOf = (config: NodeDoc): string =>
  typeof config.location === "string" ? config.location : "";

/** The patch for the location: a name writes it, "" removes it (the store follows the globe's region). */
export function setLocation(name: string, config: NodeDoc, location: string): Op[] {
  const path = ["nodes", name, "location"];
  const present = "location" in config;
  if (location === "") return present ? [{ op: "remove", path }] : [];
  return [put(path, present, location)];
}

const MULTI = new Set(["asia", "eu", "us"]);
const DUAL = new Set(["asia1", "eur4", "eur5", "eur7", "eur8", "nam4"]);

/** A location as the select shows it: Google's multi- and dual-regions say so, a region is itself. */
export function locationLabel(location: string): string {
  if (MULTI.has(location)) return `${location} (multi-region)`;
  if (DUAL.has(location)) return `${location} (dual-region)`;
  return location;
}
