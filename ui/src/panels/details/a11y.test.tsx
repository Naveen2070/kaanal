// axe-core over the right rail, the tabs' contents, the charts band, a help popover, the Connect
// menu, the command palette, the undo buttons and a tour note.

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView, RunRecord } from "../../data/api/types";
import { ConnectMenu } from "../../canvas/ConnectMenu";
import { ChartsBand } from "../charts/ChartsBand";
import { useFirstRun } from "../../overlays/first-run/store";
import { Tour } from "../../overlays/first-run/Tour";
import { HelpPopover } from "../../overlays/help/HelpPopover";
import { CommandPalette } from "../../overlays/palette/CommandPalette";
import { commands } from "../../overlays/palette/commands";
import { useDraft } from "../../data/store/draft";
import { useRun } from "../../data/store/run";
import { expectNoViolations } from "../../test/axe";
import { nodeSummary } from "../../test/summary";
import { UndoRedo } from "../../shell/topbar/UndoRedo";
import { EMPTY_FEED, FeedContext } from "../../canvas/feed";
import { Controls } from "./Controls";
import { Rail } from "./Rail";
import { Results } from "./Results";
import { LogSkeleton, ResultsSkeleton } from "./Skeletons";

const view: DraftView = {
  text: "",
  dirty: false,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc: {
    nodes: {
      gw: { type: "gateway" },
      app: { type: "service" },
      assets: { type: "storage", first_byte: "20ms", throughput: "80MB/s" },
    },
    edges: [],
  },
  doc_error: null,
  project_id: "p",
  project_name: null,
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
};

const run = {
  id: "s1",
  state: "done",
  summary: {
    label: "x",
    window_s: 1,
    requests: 10,
    failed: 0,
    errors: {},
    warnings: ["a warning"],
    latency: { p50_ms: 1, p95_ms: 2, p99_ms: 3, max_ms: 4, buckets: [{ le_ms: 1, count: 1 }] },
    series: { window_s: 1, done: [1, 2], failed: [0, 0], p99_ms: [3, 4] },
    streams: [],
    nodes: [nodeSummary({ name: "app" })],
    links: [],
    bottlenecks: [0],
  },
} as unknown as RunRecord;

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem("kaanal.studio.charts.open", "1");
  history.replaceState(null, "", "/__kaanal/studio?node=app");
  useDraft.setState({ view, error: null, restarted: false });
  useRun.setState({ status: "done", latest: run, dimmed: false, lastError: null });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("the_rail_with_its_tabs_and_sentence_has_no_axe_violations", async () => {
  const { container } = render(
    <NuqsAdapter>
      <Rail
        sentence="It gets 1,000, so it is 78% busy."
        panels={{ controls: <p>c</p>, log: <p>l</p>, results: <p>r</p>, live: <p>v</p> }}
      />
    </NuqsAdapter>,
  );
  await expectNoViolations(container);
});

test("results_charts_and_skeletons_have_no_axe_violations", async () => {
  const { container } = render(
    <NuqsAdapter>
      <Results />
      <ChartsBand />
      <ResultsSkeleton />
      <LogSkeleton />
    </NuqsAdapter>,
  );
  await expectNoViolations(container);
});

test("a_help_popover_open_has_no_axe_violations", async () => {
  const { container } = render(<HelpPopover id="p99" />);
  fireEvent.click(screen.getByRole("button", { name: "About p99" }));
  await screen.findByRole("dialog");
  await expectNoViolations(container);
  await expectNoViolations(document.body);
});

test("the_connect_menu_undo_buttons_and_tour_have_no_axe_violations", async () => {
  useFirstRun.setState({ phase: "tour", projectKey: "p" });
  const { container } = render(
    <>
      <ConnectMenu from="gw" />
      <UndoRedo />
      <Tour />
    </>,
  );
  await expectNoViolations(container);
});

test("the_command_palette_has_no_axe_violations", async () => {
  const noop = () => {};
  const { container } = render(
    <CommandPalette
      all={commands({ add: noop, run: noop, press: noop, tour: noop, fit: noop, traffic: noop })}
      onClose={noop}
    />,
  );
  await expectNoViolations(container);
  expect(screen.getByRole("combobox")).toBeTruthy();
});

test("the_controls_of_a_storage_node_have_no_axe_violations", async () => {
  const noop = () => {};
  const { container } = render(
    <FeedContext.Provider value={EMPTY_FEED}>
      <Controls
        name="assets"
        config={{
          type: "storage",
          first_byte: "20ms",
          throughput: "80MB/s",
          rate_limit: { rps: 5 },
        }}
        edges={[{ from: "app", to: "assets" }]}
        {...{ onDeleted: noop, onRenamed: noop, onChaos: noop }}
      />
      <Controls
        name="assets"
        config={{
          type: "storage",
          first_byte: { p50: "25ms", p99: "100ms" },
          throughput: "80MB/s",
        }}
        edges={[]}
        {...{ onDeleted: noop, onRenamed: noop, onChaos: noop }}
      />
    </FeedContext.Provider>,
  );
  await expectNoViolations(container);
});
