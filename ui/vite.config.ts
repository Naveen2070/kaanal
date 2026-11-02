import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import {
  defineConfig,
  type ConfigEnv,
  type Plugin,
  type PluginOption,
  type UserConfig,
} from "vite-plus";
// The server embeds exactly the files the Vite manifest references (crates/kaanal-server/build.rs),
// served from one exact-match route, `/__kaanal/studio/assets/{name}`. So the build emits hashed
// files flat under `assets/`, a manifest, and nothing else; `postbuild` precompresses them and
// restores ui-dist/.gitkeep (emptyOutDir is on, so an old hash never piles up in the binary).
const KAANAL_DEV_PORT = process.env.KAANAL_DEV_PORT ?? "7777";

// theme/boot.js must run before the first paint and the CSP allows no inline script, so it is a second
// build entry (hashed, in the manifest, in the asset table) and this plugin puts its <script> first
// in <head>. Under `vp dev` it points at the source file.
function themeBoot(): Plugin {
  return {
    name: "kaanal-theme-boot",
    transformIndexHtml(_html, ctx) {
      const chunk = ctx.bundle
        ? Object.values(ctx.bundle).find((c) => c.type === "chunk" && c.name === "boot")
        : undefined;
      const src = chunk ? `/__kaanal/studio/${chunk.fileName}` : "/src/look/theme/boot.js";
      return [{ tag: "script", attrs: { src }, injectTo: "head-prepend" }];
    },
  };
}

// The return type is annotated on purpose: left inferred, `defineConfig<T extends
// ViteUserConfigExport>` compares this large object literal against the whole export union and
// TypeScript gives TS2321 "Excessive stack depth". Naming `UserConfig` short-circuits it.
export default defineConfig(({ command }: ConfigEnv): UserConfig => ({
  // React Compiler memoizes components at build time, so the code needs no useMemo/useCallback
  plugins: [
    react(),
    babel({ presets: [reactCompilerPreset({ target: "19" })] }),
    tailwindcss(),
    themeBoot(),
  ] as PluginOption[],
  // Dev mode (`pnpm dev` / `scripts/studio-dev.mjs`): the page is served by Vite, but the control
  // API is served by a real `kaanal` on another port, so `/__kaanal/*` is proxied to it. The target
  // port comes from the runner (`scripts/studio-dev.mjs` sets it) so the two never disagree.
  server: {
    proxy: {
      // `changeOrigin` is required, not cosmetic: the control guard (kaanal-server/src/control/
      // guard.rs) rejects a Host that is not loopback WITH the bound port, and the proxy would
      // otherwise forward `localhost:5173`.
      "/__kaanal": {
        target: `http://127.0.0.1:${KAANAL_DEV_PORT}`,
        changeOrigin: true,
      },
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
    // FONTS.md is formatted by dprint (the docs formatter); the two disagree on the Tamil table row
    ignorePatterns: ["node_modules", "pnpm-lock.yaml", "FONTS.md"],
  },
  test: { environment: "jsdom" },
  // `command` is "serve" | "build"; `mode` is "development" | "production" and is NOT the
  // signal here. The embedded page is served under this base; the dev server must NOT be, or its
  // own asset requests would fall into the /__kaanal proxy above and be answered by the embedded
  // bundle instead of the live code. So the base applies to the build only.
  base: command === "serve" ? "/" : "/__kaanal/studio/",
  build: {
    outDir: "../crates/kaanal-server/ui-dist",
    emptyOutDir: true,
    manifest: true,
    // the CSP has no `data:` in font-src (and img-src data: is not for fonts): every asset is a file
    assetsInlineLimit: 0,
    sourcemap: false,
    cssCodeSplit: false,
    rolldownOptions: {
      input: { main: "index.html", boot: "src/look/theme/boot.js" },
      output: {
        entryFileNames: "assets/[name]-[hash].js",
        chunkFileNames: "assets/[name]-[hash].js",
        assetFileNames: "assets/[name]-[hash][extname]",
        // React Flow is first-load but changes rarely: its own long-cache chunk
        codeSplitting: {
          groups: [{ name: "react-flow", test: /node_modules[/]@xyflow/ }],
        },
      },
    },
  },
}));
