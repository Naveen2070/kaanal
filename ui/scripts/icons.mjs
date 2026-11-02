/* eslint-disable no-console -- a command-line script prints its result */
// Renders the 180 px apple-touch-icon.png: the white mark on the brand-blue tile (#1f6fe0 in both
// themes). Run by hand after changing src/look/brand/geometry.mjs: `node scripts/icons.mjs`. No image
// library: the mark is a few shapes, so each pixel is sampled 4x4 times and the PNG is written with
// node:zlib. The output is committed (src/look/brand/apple-touch-icon.png).

import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";
import { DISC, GRID, HORIZON, SLICES, SUN_BARS, TILE } from "../src/look/brand/geometry.mjs";

const SIZE = 180;
const MARK = 0.62; // the mark fills this share of the tile, centred
const SS = 4; // samples per pixel side

/** Coverage (0..1) of the white mark at a point on the 24 grid. */
function ink(gx, gy) {
  const inDisc = (gx - DISC.cx) ** 2 + (gy - DISC.cy) ** 2 <= DISC.r ** 2 && gy <= DISC.cy;
  if (inDisc && SUN_BARS.some((b) => gy >= b.y && gy <= b.y + b.h)) return 1;
  if (Math.abs(gy - HORIZON.y) <= HORIZON.width / 2 && gx >= HORIZON.x1 && gx <= HORIZON.x2) {
    return HORIZON.opacity;
  }
  for (const s of SLICES) {
    if (gx < s.x || gx > s.x + s.w || gy < s.y || gy > s.y + s.h) continue;
    const r = s.h / 2; // fully rounded ends
    const dx = Math.max(s.x + r - gx, gx - (s.x + s.w - r), 0);
    if (dx ** 2 + (gy - (s.y + r)) ** 2 <= r ** 2) return s.opacity;
  }
  return 0;
}

export function render() {
  const tile = [1, 3, 5].map((i) => Number.parseInt(TILE.slice(i, i + 2), 16));
  const scale = (GRID * 1) / (SIZE * MARK);
  const origin = (SIZE - SIZE * MARK) / 2;
  const raw = Buffer.alloc(SIZE * (SIZE * 3 + 1));
  for (let y = 0; y < SIZE; y++) {
    raw[y * (SIZE * 3 + 1)] = 0; // filter: none
    for (let x = 0; x < SIZE; x++) {
      let a = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const gx = (x + (sx + 0.5) / SS - origin) * scale;
          const gy = (y + (sy + 0.5) / SS - origin) * scale;
          a += ink(gx, gy);
        }
      }
      a /= SS * SS;
      for (let c = 0; c < 3; c++) {
        raw[y * (SIZE * 3 + 1) + 1 + x * 3 + c] = Math.round(tile[c] * (1 - a) + 255 * a);
      }
    }
  }
  return png(raw);
}

function crc32(buf) {
  let c = ~0;
  for (const b of buf) {
    c ^= b;
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return ~c >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function png(raw) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(SIZE, 0);
  ihdr.writeUInt32BE(SIZE, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // truecolour, opaque (iOS paints transparency black)
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const out = resolve(fileURLToPath(import.meta.url), "../../src/look/brand/apple-touch-icon.png");
  writeFileSync(out, render());
  console.log(`wrote ${out} (tile ${TILE})`);
}
