// axe-core over the slice 1 components: the shell in both wide layouts, the read-only list, the
// mark, and the loader in a normal and a failed state.

import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { Loader } from "../brand/Loader";
import { Mark } from "../brand/Mark";
import { Palette } from "../canvas/Palette";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { expectNoViolations } from "../test/axe";
import { TopBar } from "../topbar/TopBar";
import { ReadonlyNodes } from "./ReadonlyNodes";
import { Shell } from "./Shell";

const view: DraftView = {
  text: "",
  dirty: true,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc: {
    nodes: { gw: { type: "gateway" }, app: { type: "service" } },
    edges: [],
    traffic: { peak: { rps: 100, journeys: [] } },
  },
  doc_error: null,
  project_id: "p",
  project_name: "petstore",
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: false,
};

beforeEach(() => {
  useDraft.setState({ view, error: null, restarted: false });
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const shell = () => (
  <Shell
    topLeft={<TopBar onRun={() => {}} />}
    top={<p>figures</p>}
    palette={<Palette onAdd={() => {}} />}
    canvas={<p>canvas</p>}
    rail={<aside aria-label="Details">rail</aside>}
    readonly={<ReadonlyNodes />}
  />
);

for (const width of [1440, 1000]) {
  test(`the_shell_has_no_axe_violations_at_${width}_px`, async () => {
    vi.stubGlobal("innerWidth", width);
    const { container } = render(shell());
    await expectNoViolations(container, { page: true });
  });
}

test("the_read_only_list_has_no_axe_violations", async () => {
  vi.stubGlobal("innerWidth", 700);
  const { container } = render(shell());
  expect(container.textContent).toContain("edit");
  await expectNoViolations(container, { page: true });
});

test("the_mark_has_no_axe_violations_hidden_or_named", async () => {
  const { container } = render(
    <>
      <Mark />
      <Mark title="Kaanal" variant="favicon" />
    </>,
  );
  await expectNoViolations(container);
});

for (const step of [2, "failed"] as const) {
  test(`the_loader_has_no_axe_violations_at_step_${step}`, async () => {
    const { container } = render(
      <Loader step={step} reason="refused" onDone={() => {}} onRetry={() => {}} />,
    );
    await expectNoViolations(container, { page: true });
  });
}

test("axe_helper_does_catch_a_nameless_button", async () => {
  // eslint-disable-next-line jsx-a11y/control-has-associated-label -- the nameless button is the point
  const { container } = render(<button type="button" />);
  await expect(expectNoViolations(container)).rejects.toThrow();
});
