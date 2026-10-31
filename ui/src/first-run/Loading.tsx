// The loading page: the mirage loader (brand/Loader.tsx) until Studio has something true to show,
// then `children` (the app). The steps follow real events: 1 until the draft arrives (`ready`), 2
// while the first run is being started, 3 while it runs. It ends when that run settles, finished or
// blocked (no `traffic:`, a draft that does not build, a failed run): the app then shows the reason
// itself. A first run still going after FIRST_RUN_CAP_MS opens the app anyway, so it never loops
// forever. A draft that cannot be loaded shows the reason inline with a Retry.

import { useEffect, useState, type ReactNode } from "react";
import { Loader, type LoaderStep } from "../brand/Loader";
import { getMinLoadMs } from "../settings";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { usePrefersReducedMotion } from "./motion";
import { QUOTES } from "./quotes";

export const FIRST_RUN_CAP_MS = 15000;
/** Step 1 waits this long for the draft before it says the server is not answering. */
export const DRAFT_STALL_MS = 10000;
const STALL_REASON = "kaanal has not answered for 10 s. Is it still running?";

export function Loading({
  ready,
  children,
  quotes = QUOTES,
  random = Math.random,
  minMs = getMinLoadMs(),
}: {
  /** The draft has loaded. */
  ready: boolean;
  children: ReactNode;
  quotes?: string[];
  /** Where the quotes come from; a test passes a fixed one. */
  random?: () => number;
  /** The loader stays at least this long (a setting, 10 s by default) so it is seen, not flashed. */
  minMs?: number;
}) {
  const error = useDraft((s) => s.error);
  const load = useDraft((s) => s.load);
  const status = useRun((s) => s.status);
  const settled = useRun((s) => s.latest !== null || s.status === "done" || s.status === "blocked");
  const [capped, setCapped] = useState(false);
  const [minElapsed, setMinElapsed] = useState(false);
  const still = usePrefersReducedMotion();
  // no hold for reduced motion (a still picture gains nothing from waiting) or a hidden tab
  const hold = still || document.hidden ? 0 : minMs;
  const [revealed, setRevealed] = useState(false);
  const [stalled, setStalled] = useState(false);
  /** A Retry in flight: back to step 1, and a second click cannot start a parallel load. */
  const [retrying, setRetrying] = useState(false);

  useEffect(() => {
    if (hold <= 0) return;
    const t = setTimeout(() => setMinElapsed(true), hold);
    return () => clearTimeout(t);
  }, [hold]);

  useEffect(() => {
    if (!ready) return;
    const t = setTimeout(() => setCapped(true), FIRST_RUN_CAP_MS);
    return () => clearTimeout(t);
  }, [ready]);

  // `getDraft` has no timeout: a server that accepts the connection and never answers is named
  useEffect(() => {
    if (ready || (error && !retrying)) return;
    const t = setTimeout(() => setStalled(true), DRAFT_STALL_MS);
    return () => clearTimeout(t);
  }, [ready, error, retrying]);

  const retry = () => {
    setRetrying(true);
    setStalled(false);
    void load().finally(() => setRetrying(false));
  };

  if (revealed) return <>{children}</>;
  const step: LoaderStep = !ready
    ? stalled || (error && !retrying)
      ? "failed"
      : 1
    : settled || capped
      ? hold > 0 && !minElapsed
        ? 3 // all done early: stay on the last true step until the hold ends
        : "ready"
      : status === "running"
        ? 3
        : 2;
  return (
    <Loader
      step={step}
      reason={stalled ? STALL_REASON : (error ?? undefined)}
      onDone={() => setRevealed(true)}
      onRetry={retry}
      quotes={quotes}
      random={random}
    />
  );
}
