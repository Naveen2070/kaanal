// The loader (spec 12): compositions by width, shuffled quotes, discrete steps with one live region,
// the reveal and its cuts, the failed state, the composited bands, the outlined wordmark.

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { DISC, SUN_BARS } from "./geometry.mjs";
import { Loader, nextQuote, pickComposition } from "./Loader";
import { PALETTES, PALM_CLEARANCE, sceneFile } from "./Scene";
import { WORDMARK } from "./wordmark";

const ANIMATED = ".kaanal-shimmer, .kaanal-dash, .kaanal-fade, .kaanal-settle";

function reduceMotion() {
  vi.stubGlobal("matchMedia", (q: string) => ({
    matches: q.includes("reduce"),
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

function hideTab(hidden: boolean) {
  Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  hideTab(false);
});

describe("Loader", () => {
  it("pickComposition_boundaries", () => {
    expect(pickComposition(899)).toBe("medium");
    expect(pickComposition(900)).toBe("wide");
    expect(pickComposition(599)).toBe("narrow");
    expect(pickComposition(600)).toBe("medium");
    expect(pickComposition(0)).toBe("narrow");
    expect(pickComposition(2560)).toBe("wide");
  });

  it("nextQuote_never_repeats", () => {
    for (const n of [2, 3, 14, 30]) {
      for (let prev = 0; prev < n; prev++) {
        for (const r of [0, 0.1, 0.5, 0.9, 0.999999]) {
          const next = nextQuote(prev, n, () => r);
          expect(next).not.toBe(prev);
          expect(next).toBeGreaterThanOrEqual(0);
          expect(next).toBeLessThan(n);
        }
      }
    }
    // every other quote can come up
    const seen = new Set([0, 0.2, 0.4, 0.6, 0.8].map((r) => nextQuote(2, 6, () => r)));
    expect([...seen].sort((a, b) => a - b)).toEqual([0, 1, 3, 4, 5]);
    expect(nextQuote(0, 1, () => 0.5)).toBe(0);
  });

  it("loader_status_is_discrete_steps_no_percent", () => {
    render(<Loader step={2} onDone={() => {}} />);
    expect(screen.getAllByText(/Step 2 of 3 · Reading your draft/).length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toContain("%");
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("live_region_is_on_the_step_line_only", () => {
    render(<Loader step={1} onDone={() => {}} quotes={["A quote that stays quiet."]} />);
    const live = screen.getAllByRole("status");
    expect(live).toHaveLength(1);
    expect(live[0]!.textContent).toBe(
      "Kaanal Studio is loading. Step 1 of 3 · Starting the simulator",
    );
    const quote = screen.getByText("A quote that stays quiet.");
    expect(quote.closest("[role=status], [aria-live]")).toBeNull();
    expect(document.querySelectorAll("[aria-live]")).toHaveLength(0); // role=status is the one
  });

  it("reduced_motion_renders_still_art_and_cuts", () => {
    reduceMotion();
    const onDone = vi.fn();
    const { rerender } = render(<Loader step={3} onDone={onDone} />);
    expect(document.querySelector(ANIMATED)).toBeNull();
    rerender(<Loader step="ready" onDone={onDone} />);
    expect(onDone).toHaveBeenCalledTimes(1);
    expect(document.querySelector(ANIMATED)).toBeNull();
  });

  it("with_motion_the_water_shimmers_and_ready_plays_the_reveal", async () => {
    const onDone = vi.fn();
    const { rerender } = render(<Loader step={3} onDone={onDone} />);
    expect(document.querySelectorAll(".kaanal-shimmer")).toHaveLength(15);
    rerender(<Loader step="ready" onDone={onDone} />);
    expect(document.querySelectorAll(".kaanal-shimmer")).toHaveLength(0);
    expect(document.querySelector(".kaanal-settle")).not.toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(800));
    expect(onDone).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTimeAsync(200));
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("hidden_tab_at_ready_cuts_straight_to_the_app", () => {
    hideTab(true);
    const onDone = vi.fn();
    render(<Loader step="ready" onDone={onDone} />);
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("failed_step_shows_the_reason_inline_and_a_retry", () => {
    const onRetry = vi.fn();
    const onDone = vi.fn();
    render(
      <Loader
        step="failed"
        reason="the server stopped answering"
        onDone={onDone}
        onRetry={onRetry}
      />,
    );
    expect(screen.getByText("Studio could not start: the server stopped answering")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain("the server stopped answering");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(onDone).not.toHaveBeenCalled();
    // a failure is not animated: the person reads it
    expect(document.querySelector(".kaanal-shimmer")).toBeNull();
    expect(document.querySelector(".kaanal-dash")).toBeNull();
    // and the live region is not inside a busy subtree, where assistive technology may hold it back
    expect(document.querySelector("[aria-busy='true']")).toBeNull();
  });

  it("bands_are_fifteen_composited_divs", () => {
    render(<Loader step={1} onDone={() => {}} />);
    const bands = document.querySelectorAll("[data-band]");
    expect(bands).toHaveLength(15);
    for (const b of bands) {
      expect(b.tagName).toBe("DIV");
      expect((b as HTMLElement).style.overflow).toBe("hidden");
      expect(b.querySelector("img")).not.toBeNull();
    }
    expect(document.querySelector("use")).toBeNull();
  });

  it("the_reflection_colours_are_the_tokens", () => {
    const css =
      readFileSync("src/look/theme/tokens.css", "utf8") + readFileSync("src/main.css", "utf8");
    const pair = (name: string) => {
      const m = new RegExp(String.raw`--${name}:\s*light-dark\(\s*(#\w+)\s*,\s*(#\w+)\s*\)`).exec(
        css,
      );
      expect(m, `--${name} in tokens.css or main.css`).not.toBeNull();
      return [m![1], m![2]] as const;
    };
    for (const [i, theme] of (["light", "dark"] as const).entries()) {
      expect(PALETTES[theme]).toEqual({
        sun: pair("accent")[i],
        ink: pair("text")[i],
        art: pair("kl-art")[i],
        sky: pair("kl-sky")[i],
      });
    }
  });

  it("the_reflection_images_are_current_and_the_palms_clear_the_k", () => {
    for (const comp of ["wide", "medium", "narrow"] as const) {
      for (const theme of ["light", "dark"] as const) {
        const file = readFileSync(`src/look/brand/scene/${comp}-${theme}.svg`, "utf8");
        expect(file, `${comp}-${theme}: run node scripts/scene.mjs`).toBe(
          sceneFile(comp, PALETTES[theme], WORDMARK),
        );
        expect(file).not.toMatch(/<(text|use)\b/);
      }
    }
    expect(PALM_CLEARANCE).toBeGreaterThanOrEqual(24);
  });

  it("the_page_placeholder_is_the_ground_and_the_mark_only", () => {
    const html = readFileSync("index.html", "utf8");
    const root = /<div id="root">([\s\S]*?)<\/div>\s*<script/.exec(html)![1]!;
    // the mark's numbers (geometry.mjs), written out by hand: no script may draw it
    const r = (v: number) => +v.toFixed(2);
    expect(root).toContain(
      `d="M${r(DISC.cx - DISC.r)} ${DISC.cy}A${DISC.r} ${DISC.r} 0 0 1 ${r(DISC.cx + DISC.r)} ${DISC.cy}Z"`,
    );
    for (const b of SUN_BARS) expect(root).toContain(`y="${b.y}" width="20" height="${b.h}"`);
    expect(root).not.toMatch(/<(text|img|p|script)\b/);
    // inline style is allowed by the CSP, inline script is not
    expect(html).not.toMatch(/<script(?![^>]*\bsrc=)[^>]*>/);
  });

  it("wordmark_is_a_path_not_text", () => {
    render(<Loader step={1} onDone={() => {}} />);
    const mark = document.querySelector("svg path[data-wordmark]");
    expect(mark).not.toBeNull();
    expect(mark!.getAttribute("d")).toBe(WORDMARK.d);
    expect(WORDMARK.d).toMatch(/^M[-\d.]/);
    expect(WORDMARK.width).toBeGreaterThan(0);
    expect(document.querySelector("text")).toBeNull();
    // the gloss is real text, in its languages
    expect(document.body.querySelector('[lang="ta"]')?.textContent).toBe("கானல்");
    expect(document.body.querySelector('[lang="en"]')?.textContent).toBe("mirage");
  });
});
