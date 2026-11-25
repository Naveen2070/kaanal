import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import react from "@astrojs/react";
import starlightLinksValidator from "starlight-links-validator";

// Served from the domain root (no `base`); docs live under /docs, Why Kaanal is /docs/.
export default defineConfig({
  site: "https://kaanal.naveenr.in",
  integrations: [
    starlight({
      title: "Kaanal",
      description: "A single binary that pretends to be your backend: mocks, simulation and chaos.",
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/Naveen2070/kaanal" }],
      customCss: ["./src/styles/kaanal.css"],
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
