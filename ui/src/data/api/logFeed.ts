// The request feed of Admin mode (step 8A): the mock's finished requests, polled once a second
// from `GET /__kaanal/logs?after=<last>`. Nothing polls in Live or Batch, and nothing while the page
// is hidden, so the log ring is read only when someone is looking.

import { api } from "./client";

/** One finished mock request, as the log ring keeps it (the fields the feed shows). */
export interface FeedRow {
  id: string;
  /** Completion order: the cursor of the next poll. */
  done: number;
  ts_ms: number;
  method: string;
  path: string;
  endpoint: string | null;
  status: number;
  /** Measured wall time, ms. */
  ms: number;
  /** A live payload edit answered it. */
  edited?: boolean;
  pin?: string | null;
  /** Refused before any chain ran (`unmatched`, `bad-scope`, ...). */
  rejected?: string | null;
}

const FIRST = 100;
const NEXT = 200;

/** Starts polling; `onRows` gets each new batch (oldest first). Returns the stop function. */
export function startFeed(
  onRows: (rows: FeedRow[]) => void,
  opts: { everyMs?: number } = {},
): () => void {
  let last: number | null = null;
  let stopped = false;
  let busy = false;
  const poll = async () => {
    if (stopped || busy || document.hidden) return;
    busy = true;
    try {
      const q = last === null ? { limit: FIRST } : { after: last, limit: NEXT };
      const rows = (await api.getFeed(q)).entries;
      if (stopped) return;
      if (rows.length > 0) last = rows[rows.length - 1]?.done ?? last;
      // an empty first answer still moves the cursor, or every poll would be a "first" one
      if (last === null) last = 0;
      onRows(rows);
    } catch {
      // a failed poll is skipped; the next one asks again from the same place
    } finally {
      busy = false;
    }
  };
  void poll();
  const timer = setInterval(() => void poll(), opts.everyMs ?? 1000);
  return () => {
    stopped = true;
    clearInterval(timer);
  };
}
