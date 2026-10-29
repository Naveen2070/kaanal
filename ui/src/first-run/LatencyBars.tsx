// The loader: six bars rising and falling, like a latency histogram breathing. Under
// `prefers-reduced-motion` it is one still frame.

import { useEffect, useState } from "react";

const HEIGHTS = [0.45, 0.8, 0.6, 1, 0.7, 0.5];

/** True when the person asked their system for less motion. Absent `matchMedia` (a test) is false. */
export function usePrefersReducedMotion(): boolean {
  const query = "(prefers-reduced-motion: reduce)";
  const [reduced, setReduced] = useState(() =>
    typeof window.matchMedia === "function" ? window.matchMedia(query).matches : false,
  );
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia(query);
    const on = (e: MediaQueryListEvent) => setReduced(e.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return reduced;
}

export function LatencyBars() {
  const still = usePrefersReducedMotion();
  return (
    <div
      aria-hidden
      data-motion={still ? "off" : "on"}
      className="flex h-10 items-end gap-1 text-neutral-500"
    >
      {HEIGHTS.map((h, i) => (
        <span
          key={i}
          className={`w-2 origin-bottom rounded-sm bg-current ${still ? "" : "kaanal-bar"}`}
          style={{
            height: "100%",
            transform: still ? `scaleY(${h})` : undefined,
            animationDelay: still ? undefined : `${i * 0.12}s`,
          }}
        />
      ))}
    </div>
  );
}
