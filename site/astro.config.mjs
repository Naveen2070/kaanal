import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import react from "@astrojs/react";
import starlightLinksValidator from "starlight-links-validator";

// Untitled code blocks get a title from their language so every block reads like an editor window
// ("Terminal" for shell blocks, "YAML", "JSON", "HTTP"..., "Output" for untagged text; any other language
// shows its tag in capitals). A block with its own title="file.yaml" keeps it.
const BLOCK_TITLE = {
  bash: "Terminal", sh: "Terminal", shell: "Terminal", zsh: "Terminal", console: "Terminal",
  powershell: "PowerShell", yaml: "YAML", yml: "YAML", json: "JSON", http: "HTTP",
  ts: "TypeScript", typescript: "TypeScript", js: "JavaScript", toml: "TOML", rust: "Rust",
  text: "Output", txt: "Output", plaintext: "Output",
};
const defaultBlockTitle = {
  name: "kaanal-default-block-title",
  hooks: {
    preprocessCode: ({ codeBlock }) => {
      if (codeBlock.props.title) return;
      // a block with no language tag is plain text (traces, sample output); any other language is named by its tag
      const lang = codeBlock.language || "text";
      codeBlock.props.title = BLOCK_TITLE[lang] ?? lang.toUpperCase();
    },
  },
};

// Served from the domain root (no `base`); docs live under /docs, Why Kaanal is /docs/.
export default defineConfig({
  site: "https://kaanal.naveenr.in",
  integrations: [
    starlight({
      title: "Kaanal",
      description: "A single binary that pretends to be your backend: mocks, simulation and chaos.",
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/Naveen2070/kaanal" }],
      customCss: ["./src/styles/kaanal.css"],
      // Preload both fonts so a page change does not paint in the fallback font and then swap (that moved the top bar).
      head: ["inter-latin", "jetbrains-mono-latin"].map((f) => ({
        tag: "link",
        attrs: { rel: "preload", as: "font", type: "font/woff2", href: `/fonts/${f}.woff2`, crossorigin: "anonymous" },
      })),
      // Frames: shell blocks get the macOS-style terminal header (three dots), titled blocks a file tab. One
      // radius and one border for the whole frame; the old CSS drew the header and the code box separately.
      expressiveCode: {
        plugins: [defaultBlockTitle],
        styleOverrides: {
          borderRadius: "0.75rem",
          borderColor: "var(--kaanal-border)",
          codeFontSize: "0.85rem",
          codePaddingBlock: "0.85rem",
          codePaddingInline: "1rem",
          frames: {
            frameBoxShadowCssValue: false,
            editorTabBarBackground: "var(--kaanal-raised)",
            editorTabBarBorderBottomColor: "var(--kaanal-border)",
            editorActiveTabBackground: "var(--kaanal-well)",
            editorActiveTabForeground: "var(--kaanal-text)",
            editorActiveTabIndicatorTopColor: "var(--kaanal-accent)",
            editorActiveTabIndicatorBottomColor: "transparent",
          },
        },
      },
      components: {
        SiteTitle: "./src/components/starlight/SiteTitle.astro",
        Header: "./src/components/starlight/Header.astro",
        Sidebar: "./src/components/starlight/Sidebar.astro",
      },
      plugins: [starlightLinksValidator()],
      sidebar: [
        { label: "Why Kaanal", slug: "docs" },
        { label: "Start here", items: [{ autogenerate: { directory: "docs/start" } }] },
        {
          label: "How-to guides",
          items: [
            { slug: "docs/guides", label: "Overview" },
            { label: "Mock your backend", items: [{ autogenerate: { directory: "docs/guides/mock" } }] },
            { label: "Break it on purpose", items: [{ autogenerate: { directory: "docs/guides/break" } }] },
            { label: "Simulate and size", items: [{ autogenerate: { directory: "docs/guides/simulate" } }] },
            { label: "Observe and automate", items: [{ autogenerate: { directory: "docs/guides/observe" } }] },
          ],
        },
        { label: "How it works", items: [{ autogenerate: { directory: "docs/how-it-works" } }] },
        {
          label: "Reference",
          items: [
            { slug: "docs/reference", label: "Overview" },
            "docs/reference/kaanal-yaml",
            "docs/reference/cli",
            "docs/reference/nodes",
            "docs/reference/edges-and-failure",
            "docs/reference/resources",
            { label: "Control API", items: [{ autogenerate: { directory: "docs/reference/control-api" } }] },
          ],
        },
        { label: "Comparison", slug: "docs/comparison" },
      ],
    }),
    react(),
  ],
});
