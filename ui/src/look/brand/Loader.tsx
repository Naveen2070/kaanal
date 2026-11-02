// The loader (spec 12, the mirage): a flat sky over flat water, the picture of Scene.ts above the
// horizon, and its reflection as 15 HTML bands, each an `overflow: hidden` div showing the
// pre-rendered picture flipped and drifting sideways with `transform` only (composited, so it stays
// smooth while chunks load; SVG <use> copies were measured too slow). Below: a quote and the step.
// At "ready" the shimmer stops, the slices settle and fade (0.9 s), then `onDone`; a hidden tab or
// reduced motion cuts at once, and reduced motion draws everything still.

import { useEffect, useRef, useState, type CSSProperties } from "react";
import { QUOTES } from "../../overlays/first-run/quotes";
import { usePrefersReducedMotion } from "../../overlays/first-run/motion";
import { getTheme } from "../theme/theme";
import { pickComposition, scene, type Composition, type Palette } from "./Scene";
import { WORDMARK } from "./wordmark";
import mediumDark from "./scene/medium-dark.svg";
import mediumLight from "./scene/medium-light.svg";
import narrowDark from "./scene/narrow-dark.svg";
import narrowLight from "./scene/narrow-light.svg";
import wideDark from "./scene/wide-dark.svg";
import wideLight from "./scene/wide-light.svg";

export { pickComposition } from "./Scene";

export type LoaderStep = 1 | 2 | 3 | "ready" | "failed";

const STEPS = ["Starting the simulator", "Reading your draft", "Running the first simulation"];
const REVEAL_MS = 900;
const ROTATE_MS = 4000;
const BANDS = 15;
/** Band height in picture units; the first starts 3 below the horizon. The bands abut: the only gaps
 *  in the water are the sun's own (mirrored), so the reflection has the sun's rhythm, not extra seams. */
const BAND_H = 10;
const BAND_TOP = 3;

const IMAGES: Record<Composition, Record<"light" | "dark", string>> = {
  wide: { light: wideLight, dark: wideDark },
  medium: { light: mediumLight, dark: mediumDark },
  narrow: { light: narrowLight, dark: narrowDark },
};

/** The inline picture follows the theme through the tokens. */
const LIVE: Palette = {
  sun: "var(--accent)",
  ink: "var(--text)",
  art: "var(--kl-art)",
  sky: "var(--kl-sky)",
};

/** A quote index other than `prev` (for n > 1), each of the others equally likely. */
export function nextQuote(prev: number, n: number, rand: () => number): number {
  if (n <= 1) return 0;
  const j = Math.min(Math.floor(rand() * (n - 1)), n - 2);
  return j >= prev ? j + 1 : j;
}

function darkNow(): boolean {
  const t = getTheme();
  if (t !== "system") return t === "dark";
  return typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)").matches
    : false;
}

function useWidth(): number {
  const [w, setW] = useState(() => window.innerWidth);
  useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
}

export function Loader({
  step,
  reason,
  onDone,
  onRetry,
  quotes = QUOTES,
  random = Math.random,
}: {
  step: LoaderStep;
  /** Why it failed, shown inline at `step="failed"`. */
  reason?: string;
  onDone: () => void;
  onRetry?: () => void;
  quotes?: string[];
  /** Where the quotes come from; a test passes a fixed one. */
  random?: () => number;
}) {
  const still = usePrefersReducedMotion();
  const comp = pickComposition(useWidth());
  const [index, setIndex] = useState(() => Math.floor(random() * quotes.length));
  const ready = step === "ready";
  const failed = step === "failed";
  const moving = !still && !failed && !ready;
  // the latest onDone without restarting the reveal when the parent re-renders
  const done = useRef(onDone);
  useEffect(() => {
    done.current = onDone;
  });

  useEffect(() => {
    if (!ready) return;
    if (still || document.hidden) {
      done.current();
      return;
    }
    const t = setTimeout(() => done.current(), REVEAL_MS);
    return () => clearTimeout(t);
  }, [ready, still]);

  useEffect(() => {
    if (ready || failed) return;
    const t = setInterval(() => setIndex((i) => nextQuote(i, quotes.length, random)), ROTATE_MS);
    return () => clearInterval(t);
  }, [ready, failed, quotes.length, random]);

  const pic = scene(comp, LIVE, WORDMARK, moving);
  const image = IMAGES[comp][darkNow() ? "dark" : "light"];
  const box = { width: `min(100%, ${pic.w}px)`, aspectRatio: `${pic.w} / ${pic.h}` };
  const line = typeof step === "number" ? `Step ${step} of 3 · ${STEPS[step - 1]}` : "Ready";
  const status = failed
    ? `Kaanal Studio could not start. ${reason ?? ""}`.trim()
    : `Kaanal Studio is loading. ${line}`;

  return (
    <main
      className={`kaanal-loader fixed inset-0 flex flex-col overflow-hidden ${ready && !still ? "kaanal-settle" : ""}`}
    >
      {/* the one live region: the step line, never the quote */}
      <output className="sr-only">{status}</output>
      <div className="relative flex-[3] bg-(--kl-sky)">
        <div aria-hidden className="absolute bottom-0 left-1/2 -translate-x-1/2" style={box}>
          <svg
            viewBox={`0 0 ${pic.w} ${pic.h}`}
            className="absolute inset-0 h-full w-full"
            // a constant built from our own geometry, not user input
            dangerouslySetInnerHTML={{ __html: pic.svg }}
          />
          <p
            className="absolute m-0 -translate-y-full text-sm whitespace-nowrap text-muted"
            style={{
              left: `${(pic.gloss.x / pic.w) * 100}%`,
              top: `${(pic.gloss.y / pic.h) * 100}%`,
            }}
          >
            <span lang="ta" style={{ fontFamily: '"Noto Sans Tamil", var(--font-sans)' }}>
              கானல்
            </span>
            {" · "}
            <span lang="en">mirage</span>
          </p>
        </div>
      </div>
      <div className="relative flex-[2] border-t border-(--kl-art) bg-(--kl-water)">
        <div aria-hidden className="absolute top-0 left-1/2 -translate-x-1/2" style={box}>
          {Array.from({ length: BANDS }, (_, i) => {
            const top = BAND_TOP + i * BAND_H;
            return (
              <div
                key={i}
                data-band={i}
                className="absolute inset-x-0"
                style={{
                  overflow: "hidden",
                  top: `${(top / pic.h) * 100}%`,
                  // +0.5px: abutting bands would show a hairline where the rounding leaves a gap
                  height: `calc(${(BAND_H / pic.h) * 100}% + 0.5px)`,
                  opacity: +(0.92 - i * 0.055).toFixed(2),
                }}
              >
                {/* the band's slice of the flipped picture: the image's bottom edge meets the horizon */}
                <img
                  src={image}
                  alt=""
                  className={`absolute left-0 w-full max-w-none ${moving ? "kaanal-shimmer" : ""}`}
                  style={
                    {
                      top: `${(-top / BAND_H) * 100}%`,
                      height: `${(pic.h / BAND_H) * 100}%`,
                      "--a": (1 + i * 0.85).toFixed(1),
                      animationDelay: `${-i * 0.41}s`,
                    } as CSSProperties
                  }
                />
              </div>
            );
          })}
        </div>
        <div className="absolute inset-x-0 bottom-0 flex flex-col items-center gap-4 px-6 pb-8 text-center">
          <p
            key={index}
            className={`m-0 min-h-14 max-w-xl text-lg text-text sm:text-xl ${still ? "" : "kaanal-fade"}`}
          >
            {quotes[index]}
          </p>
          {failed ? (
            <div className="flex flex-wrap items-center justify-center gap-3 text-sm">
              <p className="m-0 text-text">Studio could not start: {reason ?? "no reason given"}</p>
              {onRetry ? (
                <button
                  type="button"
                  onClick={onRetry}
                  className="h-8 min-w-8 rounded-md border border-control bg-sheet px-3 text-text hover:bg-raised focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                >
                  Retry
                </button>
              ) : null}
            </div>
          ) : (
            <p aria-hidden className="m-0 text-sm text-muted tabular-nums">
              {line}
            </p>
          )}
        </div>
      </div>
    </main>
  );
}
