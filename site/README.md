# Kaanal docs site

Starlight (Astro) site for <https://kaanal.naveenr.in>. Pages are Markdown in `src/content/docs/docs/`.

```sh
pnpm install
pnpm dev      # http://localhost:4321
pnpm build    # fails on a broken internal link or anchor; writes dist/ and the search index
```

- Add a page: a `.md` file with `title` (and `sidebar.order`) in the right folder under `src/content/docs/docs/`; the sidebar lists the folder, except Reference, which is a hand-written list in `astro.config.mjs` (add the page there).
- `pnpm dev` and `pnpm build` first copy `crates/kaanal-core/kaanal.schema.json` to `public/schema/v1.json` (git-ignored); `kaanal init` and the demos point editors at `https://kaanal.naveenr.in/schema/v1.json`.
- Fonts and colours come from Studio (`ui/FONTS.md`, `ui/src/look/theme/tokens.css`).
- Deploy: `.github/workflows/site.yml` (see `docs/source/how-to.md`, "Docs site").
