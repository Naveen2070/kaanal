// Chrome flags a form field with neither an id nor a name (autofill, some a11y checks). Render the
// Studio's panels and check that none is left without one. The Log tab's own controls are not here.

import { cleanup, render } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../data/api/types";
import { ControlBar } from "../canvas/ControlBar";
import { EMPTY_FEED, FeedContext } from "../canvas/feed";
import { useDraft } from "../data/store/draft";
import { useRun } from "../data/store/run";
import { Glossary } from "../overlays/guide/Glossary";
import { CommandPalette } from "../overlays/palette/CommandPalette";
import { commands } from "../overlays/palette/commands";
import { KpiStrip } from "../shell/topbar/KpiStrip";
import { Controls } from "./details/Controls";
import { EdgeControls } from "./details/EdgeControls";
import { Live } from "./details/Live";
import { Results } from "./details/Results";
import { openTrafficBuilder } from "./traffic/store";
import { TrafficBuilder } from "./traffic/TrafficBuilder";

const app = { type: "service", concurrency: 8, replicas: 3, proc: "12ms" };
const doc = {
  nodes: { gw: { type: "gateway" }, app, db: { type: "database" } },
  edges: [
    { from: "gw", to: "app" },
    { from: "app", to: "db", bandwidth: 2048 },
  ],
  traffic: { peak: { rps: 100, journeys: [] } },
};
const view = {
  text: "",
  dirty: false,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc,
  doc_error: null,
  project_id: "p",
  project_name: null,
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
} as unknown as DraftView;

beforeEach(() => {
  // jsdom has no ResizeObserver, which Radix Slider needs
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  useDraft.setState({ view, error: null, restarted: false });
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function unnamed(root: ParentNode) {
  return [...root.querySelectorAll("input, select, textarea")]
    .filter((el) => !el.id && !el.getAttribute("name"))
    .map((el) => el.outerHTML.slice(0, 120));
}

test("no_form_control_in_the_main_panels_lacks_an_id_or_name", () => {
  const noop = () => {};
  openTrafficBuilder();
  const { container } = render(
    <NuqsAdapter>
      <FeedContext.Provider value={EMPTY_FEED}>
        <KpiStrip />
        <ControlBar name="app" config={app} />
        <Controls
          name="app"
          config={app}
          edges={doc.edges}
          {...{ onDeleted: noop, onRenamed: noop, onChaos: noop }}
        />
        <EdgeControls index={1} />
        <Live />
        <Results />
        <TrafficBuilder />
        <Glossary />
        <CommandPalette
          all={commands({
            add: noop,
            run: noop,
            press: noop,
            tour: noop,
            fit: noop,
            traffic: noop,
          })}
          onClose={noop}
        />
      </FeedContext.Provider>
    </NuqsAdapter>,
  );
  expect(container.querySelectorAll("input, select, textarea").length).toBeGreaterThan(10);
  expect(unnamed(document.body)).toEqual([]);
});
