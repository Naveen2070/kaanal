// The mark (spec 12): unique ids per instance, labelled or hidden, the gap rule, the favicon master.

import { render } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vite-plus/test";
import { DISC, FAVICON, SUN_BARS } from "./geometry.mjs";
import { Mark } from "./Mark";

const gaps = (bars: { y: number; h: number }[]) =>
  bars.slice(1).map((b, i) => b.y - (bars[i]!.y + bars[i]!.h));

describe("Mark", () => {
  it("two_marks_on_one_page_have_unique_ids", () => {
    const { container } = render(
      <>
        <Mark />
        <Mark />
      </>,
    );
    const ids = [...container.querySelectorAll("[id]")].map((e) => e.id);
    expect(ids.length).toBeGreaterThanOrEqual(2);
    expect(new Set(ids).size).toBe(ids.length);
    for (const el of container.querySelectorAll("[clip-path]")) {
      const id = /url\(#([^)]+)\)/.exec(el.getAttribute("clip-path")!)![1]!;
      expect(container.querySelector(`[id="${id}"]`), `clip-path target ${id}`).not.toBeNull();
    }
  });

  it("mark_is_labelled_or_hidden", () => {
    const hidden = render(<Mark />).container.querySelector("svg")!;
    expect(hidden.getAttribute("aria-hidden")).toBe("true");
    expect(hidden.getAttribute("role")).toBeNull();

    const named = render(<Mark title="Kaanal" />).container.querySelector("svg")!;
    expect(named.getAttribute("aria-hidden")).toBeNull();
    expect(named.getAttribute("role")).toBe("img");
    expect(named.querySelector("title")!.textContent).toBe("Kaanal");
  });

  it("gaps_grow_toward_the_horizon_and_stay_inside_the_disc", () => {
    const [g1, g2] = gaps(SUN_BARS);
    expect(g1).toBeCloseTo(0.9, 5);
    expect(g2).toBeCloseTo(1.5, 5);
    expect(g2!).toBeGreaterThan(g1!);
    // the lowest bar ends on the disc's flat edge; the top bar overshoots and the clip trims it
    const last = SUN_BARS.at(-1)!;
    expect(last.y + last.h).toBeCloseTo(DISC.cy, 5);

    const { container } = render(<Mark variant="sun" />);
    const clip = container.querySelector("clipPath path")!;
    expect(clip.getAttribute("d")).toBe(
      `M${DISC.cx - DISC.r} ${DISC.cy}A${DISC.r} ${DISC.r} 0 0 1 ${DISC.cx + DISC.r} ${DISC.cy}Z`,
    );
    expect(container.querySelectorAll("g[clip-path] rect")).toHaveLength(SUN_BARS.length);
  });

  it("the_full_mark_adds_the_horizon_and_three_slices_the_sun_alone_does_not", () => {
    const full = render(<Mark variant="full" />).container;
    expect(full.querySelectorAll("line")).toHaveLength(1);
    expect(full.querySelectorAll("rect[rx]")).toHaveLength(3);
    const sun = render(<Mark variant="sun" />).container;
    expect(sun.querySelectorAll("line")).toHaveLength(0);
    expect(sun.querySelectorAll("rect[rx]")).toHaveLength(0);
  });

  it("favicon_master_has_two_bars_one_gap_no_horizon", () => {
    expect(FAVICON.bars).toHaveLength(2);
    expect(gaps(FAVICON.bars)).toEqual([1]);
    expect(FAVICON.slice.opacity).toBeGreaterThanOrEqual(0.6);

    const { container } = render(<Mark variant="favicon" />);
    expect(container.querySelector("svg")!.getAttribute("viewBox")).toBe("0 0 16 16");
    expect(container.querySelectorAll("line")).toHaveLength(0);
    expect(container.querySelectorAll("g[clip-path] rect")).toHaveLength(2);
    expect(container.querySelectorAll("rect[rx]")).toHaveLength(1);

    // the shipped file is drawn from the same numbers
    const svg = readFileSync("src/brand/favicon.svg", "utf8");
    expect(svg).toContain('viewBox="0 0 16 16"');
    expect(svg).not.toContain("<line");
    expect((svg.match(/<rect/g) ?? []).length).toBe(3);
  });

  it("the_touch_icon_is_a_180px_png", () => {
    const png = readFileSync("src/brand/apple-touch-icon.png");
    expect(png.subarray(1, 4).toString("latin1")).toBe("PNG");
    expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([180, 180]);
  });
});
