# Fonts

Self-hosted, WOFF2, all under the SIL Open Font License 1.1 (OFL). Nothing is loaded from a CDN at run time (the CSP allows `font-src 'self'` only). The source is the pinned `@fontsource*` 5.3.0 packages (OFL-1.1, published 2026-07-19), fetched once and kept in `src/look/fonts/`; nothing in `package.json`.

| File                                          | Family                                    | Use                                          | Licence | Copyright                                                                            |
| --------------------------------------------- | ----------------------------------------- | -------------------------------------------- | ------- | ------------------------------------------------------------------------------------ |
| `src/look/fonts/inter-latin.woff2`            | Inter (variable `wght`, Latin)            | all UI text; preloaded                       | OFL 1.1 | 2016 The Inter Project Authors (https://github.com/rsms/inter)                       |
| `src/look/fonts/jetbrains-mono-latin.woff2`   | JetBrains Mono (variable `wght`, Latin)   | figures, ids and trace lines; on first paint | OFL 1.1 | 2020 The JetBrains Mono Project Authors (https://github.com/JetBrains/JetBrainsMono) |
| `src/look/fonts/noto-sans-tamil-lockup.woff2` | Noto Sans Tamil (variable `wght`, subset) | the "கானல்" gloss in the lockup and loader    | OFL 1.1 | 2022 The Noto Project Authors (https://github.com/notofonts/tamil)                   |

## How the files were made

- Inter and JetBrains Mono are the Latin subsets as published by Fontsource, unchanged.
- The Tamil file is the Fontsource Tamil subset cut again to the five letters of "கானல்" plus the middle dot (U+00B7) with `pyftsubset --text="கானல்·" --layout-features='*' --flavor=woff2` (fonttools 4.68), keeping the `wght` axis and all layout features so the letters shape correctly. If the lockup text changes, redo this.

## The wordmark (no font file shipped)

`src/look/brand/wordmark.ts` is "kaanal" outlined from Inter SemiBold (OFL 1.1) as one SVG path, so the loader's name never shifts on a font swap. `opentype.js` (dev only) cannot read WOFF2 or a variable instance, so the input is the static `inter-latin-600-normal.woff` from the same Fontsource 5.3.0 release, fetched into a scratch folder and not committed:

```sh
npm pack @fontsource/inter@5.3.0 && tar -xzf fontsource-inter-5.3.0.tgz
node scripts/wordmark.mjs <that folder>/package/files/inter-latin-600-normal.woff
node scripts/scene.mjs   # the reflection images draw the same path
```

## Rules

- Figures are tabular: Inter has a `tnum` feature, used through Tailwind's `tabular-nums`. A test reads the GSUB table to keep it that way.
- First-load fonts stay at or under 60 kB (`pnpm run size`); today only Inter counts, at about 47 kB.
- A new face needs an OFL (or MIT/ISC) licence and a row here before it is added.
