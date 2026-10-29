import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import { defineConfig } from "vite-plus";

// The server embeds three fixed files (crates/kaanal-server/src/control/studio.rs), so no hashes:
// index.html, app.js and app.css land flat in ui-dist/, and the page refers to them under the
// route that serves them. emptyOutDir is off so ui-dist/.gitkeep survives (names are fixed, so
// nothing goes stale).
export default defineConfig({
  // React Compiler memoizes components at build time, so the code needs no useMemo/useCallback
  plugins: [react(), babel({ presets: [reactCompilerPreset({ target: "19" })] }), tailwindcss()],
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
  base: "/__kaanal/studio/assets/",
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
});
