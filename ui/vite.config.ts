import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import { defineConfig } from "vite-plus";

// The server embeds three fixed files (crates/kaanal-server/src/control/studio.rs), so no hashes:
// index.html, app.js and app.css land flat in ui-dist/, and the page refers to them under the
// route that serves them. emptyOutDir is off so ui-dist/.gitkeep survives (names are fixed, so
// nothing goes stale).
const KAANAL_DEV_PORT = process.env.KAANAL_DEV_PORT ?? "7777";

export default defineConfig(({ command }) => ({
  // React Compiler memoizes components at build time, so the code needs no useMemo/useCallback
  plugins: [react(), babel({ presets: [reactCompilerPreset({ target: "19" })] }), tailwindcss()],
  // Dev mode (`pnpm dev` / `scripts/studio-dev.mjs`): the page is served by Vite, but the control
  // API is served by a real `kaanal` on another port, so `/__kaanal/*` is proxied to it. The target
  // port comes from the runner (`scripts/studio-dev.mjs` sets it) so the two never disagree.
  server: {
    proxy: {
      // `changeOrigin` is required, not cosmetic: the control guard (kaanal-server/src/control/
      // guard.rs) rejects a Host that is not loopback WITH the bound port, and the proxy would
      // otherwise forward `localhost:5173`.
      "/__kaanal": { target: `http://127.0.0.1:${KAANAL_DEV_PORT}`, changeOrigin: true },
    },
  },
  // Oxlint (vp check / vp lint): correctness is an error; React's rules matter most because React
  // Compiler silently skips a component that breaks the rules of hooks.
  lint: {
    plugins: ["typescript", "unicorn", "oxc", "react", "jsx-a11y"],
    categories: { correctness: "error" },
    rules: {
      "react/rules-of-hooks": "error",
      "react/exhaustive-deps": "error",
      "typescript/no-explicit-any": "error",
      "typescript/consistent-type-imports": "error",
      "no-console": "warn",
    },
    ignorePatterns: ["node_modules", "dist"],
    options: { typeAware: true, typeCheck: true },
  },
  // Oxfmt (vp fmt / vp check): written down so a change of its defaults cannot reformat the UI.
  fmt: {
    printWidth: 100,
    singleQuote: false,
    semi: true,
    trailingComma: "all",
    ignorePatterns: ["node_modules", "pnpm-lock.yaml"],
  },
  test: { environment: "jsdom" },
  // `command` is "serve" | "build"; `mode` is "development" | "production" and is NOT the
  // signal here. The embedded page is served under this base; the dev server must NOT be, or its
  // own asset requests would fall into the /__kaanal proxy above and be answered by the embedded
  // bundle instead of the live code. So the base applies to the build only.
  base: command === "serve" ? "/" : "/__kaanal/studio/assets/",
  build: {
    outDir: "../crates/kaanal-server/ui-dist",
    emptyOutDir: false,
    assetsDir: "",
    cssCodeSplit: false,
    rolldownOptions: {
      output: {
        entryFileNames: "app.js",
        chunkFileNames: "app-[name].js",
        assetFileNames: (a) =>
          a.names.some((n) => n.endsWith(".css")) ? "app.css" : "[name][extname]",
      },
    },
  },
}));
