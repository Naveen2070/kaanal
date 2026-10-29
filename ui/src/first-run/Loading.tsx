// The loading page: a rotating quote in the middle and the latency bars at the bottom. No counter,
// no caption. It stays until the draft has loaded and for at least 600 ms, so a fast start does not
// flash it; then it shows `children` (the app).

import { useEffect, useState, type ReactNode } from "react";
import { LatencyBars, usePrefersReducedMotion } from "./LatencyBars";
import { QUOTES } from "./quotes";

const MIN_MS = 600;
const ROTATE_MS = 4000;

export function Loading({
  ready,
  children,
  quotes = QUOTES,
  random = Math.random,
}: {
  ready: boolean;
  children: ReactNode;
  quotes?: string[];
  /** Where the first quote comes from; a test passes a fixed one. */
  random?: () => number;
}) {
  const [index, setIndex] = useState(() => Math.floor(random() * quotes.length));
  const [minElapsed, setMinElapsed] = useState(false);
  const still = usePrefersReducedMotion();
  const done = ready && minElapsed;

  useEffect(() => {
    const t = setTimeout(() => setMinElapsed(true), MIN_MS);
    return () => clearTimeout(t);
  }, []);
  useEffect(() => {
    if (done) return;
    const t = setInterval(() => setIndex((i) => (i + 1) % quotes.length), ROTATE_MS);
    return () => clearInterval(t);
  }, [done, quotes.length]);

  if (done) return <>{children}</>;
  return (
    <main aria-busy="true" className="flex h-full flex-col items-center justify-between p-8">
      <output className="sr-only">Loading Studio</output>
      <div className="flex flex-1 items-center">
        {/* a new key per quote restarts the fade */}
        <p
          key={index}
          className={`max-w-xl text-center text-xl text-neutral-700 ${still ? "" : "kaanal-fade"}`}
        >
          {quotes[index]}
        </p>
      </div>
      <LatencyBars />
    </main>
  );
}
