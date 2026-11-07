// The one plain sentence a finished run opens Results with: how many requests failed and why, and
// what gives out first. Every figure is read from the run summary (`requests`, `failed`, `errors`,
// and the first limit from `breaksFirst`); a field that is missing leaves its clause out rather than
// guessing. Pure, so it is tested without drawing.

import type { Summary } from "../data/api/types";
import { breaksFirst, twoSignificant } from "./metrics";

const nf = (n: number) => n.toLocaleString("en-US", { maximumFractionDigits: 0 });

/** "74%", or "Less than 1%" for a share that rounds to zero. */
function share(failed: number, requests: number): string {
  const pct = (failed / requests) * 100;
  return pct > 0 && pct < 1 ? "Less than 1%" : `${Math.round(pct)}%`;
}

export function verdictOf(s: Summary): string | null {
  if (!(s.requests > 0)) return null;
  const first = breaksFirst(s);
  const limit = first
    ? ` ${first.node} is ${first.saturatedNow ? "full" : "the first limit"}: it fills at ${twoSignificant(first.atReqS)} req/s.`
    : "";
  if (s.failed === 0) return `All ${nf(s.requests)} requests succeeded.${limit}`;
  const kinds = Object.entries(s.errors).sort((a, b) => b[1] - a[1]);
  const top = kinds[0];
  const total = kinds.reduce((sum, [, n]) => sum + n, 0);
  const kind = top ? `${top[1] * 2 > total ? ", mostly" : ", most often"} ${top[0]}` : "";
  return `${share(s.failed, s.requests)} of requests failed${kind}.${limit}`;
}
