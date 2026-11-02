// The loader's picture above the horizon (spec 12): the big sun with flat halo steps, cut by the
// mark's own heat-haze gaps; two palms on a low mound; the outlined wordmark standing on the
// horizon; on wide screens the nodes with a request dash. One SVG string builder, used twice: inline
// in the loader with CSS-variable colours (it follows the theme), and by scripts/scene.mjs with fixed
// colours for the pre-rendered reflection images. Plain TypeScript with no imports Node cannot strip,
// so the script imports this file directly.

import { DISC, SUN_BARS } from "./geometry.mjs";

export type Composition = "wide" | "medium" | "narrow";

/** Wide at or above 900 px, medium from 600 to 899, narrow below. */
export function pickComposition(width: number): Composition {
  return width >= 900 ? "wide" : width >= 600 ? "medium" : "narrow";
}

export interface Palette {
  sun: string;
  ink: string;
  /** Node outlines and edges. */
  art: string;
  /** Inside a node: the sky shows through. */
  sky: string;
}

/** The fixed colours of the reflection images; the inline picture uses the tokens instead. */
export const PALETTES: Record<"light" | "dark", Palette> = {
  light: { sun: "#2d5cc4", ink: "#211e19", art: "#a9b8cf", sky: "#e2eaf5" },
  dark: { sun: "#7ba3ff", ink: "#ececec", art: "#424242", sky: "#242424" },
};

export interface Wordmark {
  d: string;
  width: number;
  top: number;
}

interface Layout {
  /** The picture's size; its bottom edge is the horizon. */
  w: number;
  h: number;
  sun: { x: number; base: number; r: number };
  /** Left edge of the mound the palms stand on. */
  oasis: number;
  /** Wordmark size in px per em. */
  size: number;
  nodes: boolean;
  /** Centre the palms and the name (narrow). */
  centred: boolean;
}

const LAYOUTS: Record<Composition, Layout> = {
  wide: {
    w: 1200,
    h: 318,
    sun: { x: 210, base: 318, r: 130 },
    oasis: 420,
    size: 92,
    nodes: true,
    centred: false,
  },
  medium: {
    w: 900,
    h: 318,
    sun: { x: 150, base: 318, r: 96 },
    oasis: 312,
    size: 88,
    nodes: false,
    centred: false,
  },
  // a small sun hovering above the name, the way a mirage lifts it off the ground
  narrow: {
    w: 600,
    h: 360,
    sun: { x: 300, base: 236, r: 60 },
    oasis: 0,
    size: 80,
    nodes: false,
    centred: true,
  },
};

/** Halo steps: radius over the sun's, and fill opacity. Flat, no gradient. */
const HALOS = [
  [1.58, 0.06],
  [1.29, 0.11],
] as const;

/** Leaves of a palm: angle in degrees and length as a share of the frond length. */
const LEAVES = [
  [-172, 0.95],
  [-142, 1],
  [-112, 0.9],
  [-82, 0.85],
  [-52, 1],
  [-22, 0.98],
  [6, 0.8],
] as const;

/** The two palms, relative to the mound's left edge: foot, crown x, crown height, trunk, frond. */
const PALMS = [
  { foot: 66, x: 48, top: 116, trunk: 5, frond: 54 },
  { foot: 104, x: 134, top: 84, trunk: 4, frond: 42 },
] as const;
const MOUND = { w: 140, h: 17 };
/** At least 24 px between the palms' reach and the "k" (spec 12); a little more reads better. */
export const PALM_CLEARANCE = 28;

/** How far right of the mound's edge the oasis reaches: the mound, or the farthest leaf tip. */
export const OASIS_REACH = Math.max(
  MOUND.w,
  ...PALMS.flatMap((p) =>
    LEAVES.map(([a, k]) => p.x + p.frond * k * Math.cos((a * Math.PI) / 180)),
  ),
);

const n = (v: number) => +v.toFixed(1);

function leaf(tx: number, ty: number, deg: number, len: number): string {
  const r = (deg * Math.PI) / 180;
  const [c, s] = [Math.cos(r), Math.sin(r)];
  const [ex, ey] = [tx + len * c, ty + len * s + len * 0.24];
  const [mx, my] = [tx + len * 0.5 * c, ty + len * 0.5 * s];
  return `M${n(tx)} ${n(ty)}Q${n(mx)} ${n(my - len * 0.24)} ${n(ex)} ${n(ey)}Q${n(mx)} ${n(my + len * 0.1)} ${n(tx)} ${n(ty)}Z`;
}

/** The sun's visible bands: the mark's bars (geometry.mjs) scaled to this sun, the top one tall enough for the halos. */
export function sunBars(base: number, r: number): { y: number; h: number }[] {
  const k = r / DISC.r;
  return SUN_BARS.map((b, i) => {
    const bottom = base - (DISC.cy - (b.y + b.h)) * k;
    const top = i === 0 ? base - 2 * r : base - (DISC.cy - b.y) * k;
    return { y: n(top), h: n(bottom - top) };
  });
}

export interface SceneParts {
  w: number;
  h: number;
  svg: string;
  /** Where the gloss sits: left edge and bottom, in picture units. */
  gloss: { x: number; y: number };
}

/**
 * The picture for one composition. The clip id is fixed: one inline copy per page, and each image
 * file is its own document. `dash` adds the travelling request (inline only, never in an image).
 */
export function scene(comp: Composition, p: Palette, word: Wordmark, dash = false): SceneParts {
  const L = LAYOUTS[comp];
  const { x: sx, base, r } = L.sun;
  const scale = L.size / 100;
  const wordW = word.width * scale;
  const ox = L.centred ? (L.w - (OASIS_REACH + PALM_CLEARANCE + wordW)) / 2 : L.oasis;
  const wx = ox + OASIS_REACH + PALM_CLEARANCE;
  const H = L.h;
  const out: string[] = [];

  // the sun and its halos, cut by the gaps: the bars are the clip, so the cut is truly empty
  const arc = (rr: number) =>
    `M${n(sx - rr)} ${base}A${n(rr)} ${n(rr)} 0 0 1 ${n(sx + rr)} ${base}Z`;
  const reach = r * HALOS[0][0] + 2;
  out.push(
    `<clipPath id="kl-sun">${sunBars(base, r)
      .map((b) => `<rect x="${n(sx - reach)}" y="${b.y}" width="${n(2 * reach)}" height="${b.h}"/>`)
      .join("")}</clipPath>`,
    `<g clip-path="url(#kl-sun)" fill="${p.sun}">`,
    ...HALOS.map(([k, o]) => `<path d="${arc(r * k)}" fill-opacity="${o}"/>`),
    `<path d="${arc(r)}"/></g>`,
  );

  // the oasis: a low mound and two palms leaning apart, flat ink
  out.push(
    `<g fill="${p.ink}"><path d="M${n(ox)} ${H}Q${n(ox + MOUND.w / 2)} ${H - MOUND.h} ${n(ox + MOUND.w)} ${H}Z" fill-opacity="0.88"/>`,
  );
  for (const palm of PALMS) {
    const [foot, tx, ty] = [ox + palm.foot, ox + palm.x, H - palm.top];
    out.push(
      `<path d="M${n(foot)} ${H - 8}C${n(foot + 4)} ${n(H - palm.top * 0.4)} ${n(tx - 12)} ${n(H - palm.top * 0.78)} ${n(tx)} ${n(ty)}" fill="none" stroke="${p.ink}" stroke-width="${palm.trunk}" stroke-linecap="round"/>`,
      ...LEAVES.map(([a, k]) => `<path d="${leaf(tx, ty, a, palm.frond * k)}"/>`),
      `<circle cx="${n(tx)}" cy="${ty + 3}" r="${palm.trunk * 0.8}"/>`,
    );
  }
  out.push("</g>");

  // the name, standing on the horizon
  out.push(
    `<path data-wordmark="" d="${word.d}" fill="${p.ink}" transform="translate(${n(wx)} ${H - 8}) scale(${scale})"/>`,
  );

  if (L.nodes) {
    // two nodes right of the name, the second in blue, linked to each other and off the edge
    const st = `fill="${p.sky}" stroke-width="2"`;
    out.push(
      `<rect x="930" y="${H - 80}" width="120" height="80" rx="10" ${st} stroke="${p.art}"/>`,
      `<rect x="1080" y="${H - 50}" width="100" height="50" rx="10" ${st} stroke="${p.sun}"/>`,
      `<path d="M904 ${H - 30}H930M1050 ${H - 40}H1080M1180 ${H - 25}H1200" fill="none" stroke="${p.art}" stroke-width="2" stroke-linecap="round"/>`,
    );
    if (dash) {
      out.push(
        `<path class="kaanal-dash" d="M904 ${H - 30}H930M1050 ${H - 40}H1080M1180 ${H - 25}H1200" fill="none" stroke="${p.sun}" stroke-width="2" stroke-linecap="round"/>`,
      );
    }
  }

  return {
    w: L.w,
    h: H,
    svg: out.join(""),
    gloss: { x: n(wx + 4), y: n(H - 8 - word.top * scale - 14) },
  };
}

/** A whole SVG document of the picture, for the reflection image files. */
export function sceneFile(comp: Composition, p: Palette, word: Wordmark): string {
  const s = scene(comp, p, word);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${s.w} ${s.h}" width="${s.w}" height="${s.h}">${s.svg}</svg>\n`;
}
