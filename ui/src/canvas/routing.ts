// Where a wire bends. An orthogonal wire runs out of its source, turns at one x, and runs into its
// target. Halfway is fine until a card sits in that column, and then the wire climbs through it and
// reads as going somewhere else. This picks the x of the widest clear gap nearest the middle.
// (The two horizontal runs can still cross a card in a row between the ends; routing those needs real
// path finding, which a graph this size does not justify.)

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Keep this far from a card, and from the ends. */
const MARGIN = 14;

export function bendX(sx: number, tx: number, sy: number, ty: number, boxes: Box[]): number {
  const mid = (sx + tx) / 2;
  if (tx - sx < MARGIN * 4) return mid;
  const top = Math.min(sy, ty) - MARGIN;
  const bottom = Math.max(sy, ty) + MARGIN;
  // cards that stand between the ends and cross the wire's vertical run
  const blocks = boxes
    .filter((b) => b.y < bottom && b.y + b.h > top && b.x + b.w > sx + 1 && b.x < tx - 1)
    .map((b): [number, number] => [b.x - MARGIN, b.x + b.w + MARGIN])
    .sort((p, q) => p[0] - q[0]);
  // the clear stretches of [sx + MARGIN, tx - MARGIN] that no card covers
  const gaps: [number, number][] = [];
  let at = sx + MARGIN;
  for (const [from, to] of blocks) {
    if (from > at) gaps.push([at, Math.min(from, tx - MARGIN)]);
    at = Math.max(at, to);
  }
  if (at < tx - MARGIN) gaps.push([at, tx - MARGIN]);
  const usable = gaps.filter(([a, b]) => b > a);
  if (usable.length === 0) return mid;
  // the gap whose middle is nearest the wire's own middle, bending at that middle
  const centres = usable.map(([a, b]) => (a + b) / 2);
  return centres.reduce((best, c) => (Math.abs(c - mid) < Math.abs(best - mid) ? c : best));
}
