// The cloud instance table (step 8P): every AWS, Azure and GCP type the loader accepts for
// `instance: provider/type`, read once from the server so the picker and the engine share one list.
// Estimates; a network figure of 0 means the source gives none (the card is then not limited).

import { api } from "./client";

export interface InstanceRow {
  /** `provider/type`, lower case. */
  name: string;
  vcpu: number;
  /** Memory in MiB. */
  memMib: number;
  /** Peak network, Mbit/s; 0 = no figure. */
  netMbps: number;
  /** On-demand Linux price an hour in micro-USD in the provider's default region; 0 = none. */
  priceMicro: number;
}

export const PROVIDERS: readonly { id: string; label: string }[] = [
  { id: "aws", label: "AWS" },
  { id: "azure", label: "Azure" },
  { id: "gcp", label: "Google Cloud" },
];

let cached: Promise<InstanceRow[]> | null = null;

/** The whole table, fetched once; a failed fetch is tried again on the next call. */
export function loadInstances(): Promise<InstanceRow[]> {
  cached ??= api
    .getInstances()
    .then((r) =>
      r.types.map(([name, vcpu, memMib, netMbps, priceMicro]) => ({
        name,
        vcpu,
        memMib,
        netMbps,
        priceMicro,
      })),
    )
    .catch((e: unknown) => {
      cached = null;
      throw e;
    });
  return cached;
}

/** Forgets the fetched table (tests). */
export function resetInstances(): void {
  cached = null;
}

/** The row named `name` (case and spaces ignored), if the table has it. */
export function findInstance(rows: readonly InstanceRow[], name: string): InstanceRow | undefined {
  const key = name.trim().toLowerCase();
  return rows.find((r) => r.name === key);
}

/** What an instance fills, in one line: `2 vCPU, 8 GiB, up to 10 Gbit/s`. */
export function instanceLine(r: InstanceRow): string {
  const gib = Number((r.memMib / 1024).toFixed(2));
  const net =
    r.netMbps > 0 ? `up to ${Number((r.netMbps / 1000).toFixed(2))} Gbit/s` : "network not limited";
  return `${r.vcpu} vCPU, ${gib} GiB, ${net}`;
}

/** The names of a provider that contain `text`, at most `limit` of them (a datalist stays small). */
export function matching(
  rows: readonly InstanceRow[],
  provider: string,
  text: string,
  limit = 200,
): InstanceRow[] {
  const prefix = `${provider}/`;
  const q = text
    .trim()
    .toLowerCase()
    .replace(/^[a-z]+\//, "");
  const out: InstanceRow[] = [];
  for (const r of rows) {
    if (!r.name.startsWith(prefix) || !r.name.slice(prefix.length).includes(q)) continue;
    out.push(r);
    if (out.length >= limit) break;
  }
  return out;
}
