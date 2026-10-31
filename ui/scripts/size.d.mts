export interface Budget {
  /** First-load JS + CSS, gzip, in kB (1 kB = 1024 bytes). */
  jsCssGzipKb: number;
  /** First-load fonts (already compressed, counted raw), in kB. */
  fontsKb: number;
  /** A file name containing one of these words counts as a first-load font. */
  firstLoadFonts: string[];
}

export interface Report {
  ok: boolean;
  jsCssGzipKb: number;
  fontsKb: number;
  problems: string[];
}

export const BUDGET: Budget;
export function check(dir: string, budget?: Budget): Report;
