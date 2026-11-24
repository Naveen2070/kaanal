// How the cheapest search ended (step 9c-3), worded as the report's search block: why it stopped,
// runs of the cap, time used of the limit, candidates refused for size, and the record's own label.
// The word "cheapest" only ever comes from that label, which carries the limit wording.

import type { SearchInfo } from "../../data/api/types.plan";
import { clean } from "./planMath";

const STOP: Record<SearchInfo["stop"], string> = {
  exhausted: "it ran out of candidates to try",
  run_cap: "it reached the run cap, so cheaper designs may be left untried",
  time_limit: "its time limit passed, so cheaper designs may be left untried",
};

/** A span as the report prints it (humantime): `31s 800ms`, `1m 15s`, `0s`. */
export function span(ms: number): string {
  const parts: string[] = [];
  for (const [unit, size] of [
    ["h", 3_600_000],
    ["m", 60_000],
    ["s", 1000],
    ["ms", 1],
  ] as const) {
    const n = Math.floor(ms / size);
    ms -= n * size;
    if (n > 0) parts.push(`${n}${unit}`);
  }
  return parts.length > 0 ? parts.join(" ") : "0s";
}

/** What the time limit is, as the report says it: the whole limit, this search's share of it (provider sections ran), or a section's share. */
export type TimeShare = { kind: "search"; timeLimitS: number } | { kind: "section" };

const limitWords = (share: TimeShare | undefined): string =>
  share === undefined
    ? "the time limit"
    : share.kind === "search"
      ? `this search's share of the ${share.timeLimitS} s time limit, which is split between it and each provider section`
      : "this section's share of the time limit";

export function PlanSearch({
  search,
  cheapestNote,
  share,
}: {
  search: SearchInfo | undefined;
  cheapestNote?: string;
  /** Left out when no provider section ran: the limit is then the whole time limit. */
  share?: TimeShare;
}) {
  if (!search && !cheapestNote) return null;
  return (
    <div className="text-xs">
      {search ? (
        <ul className="space-y-0.5">
          <li>{`Stopped because ${STOP[search.stop]}.`}</li>
          <li>{`Runs: ${search.runs} of ${search.run_cap} allowed.`}</li>
          <li>{`Time: ${span(search.elapsed_ms)} used of ${span(search.limit_ms)}, ${limitWords(share)}.`}</li>
          {search.notes.map((n, i) => (
            <li key={i} className="[overflow-wrap:anywhere]">
              {`Refused for size: ${clean(n)}`}
            </li>
          ))}
        </ul>
      ) : null}
      {cheapestNote ? <p className="mt-1 text-muted">{clean(cheapestNote)}</p> : null}
    </div>
  );
}
