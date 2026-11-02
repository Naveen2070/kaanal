import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { mockFetch } from "../../test/fetch";
import { TrafficMix } from "./TrafficMix";

function view(doc: DraftView["doc"]): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc,
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
  };
}

const mixed = view({
  nodes: {},
  traffic: {
    peak: {
      rps: 300,
      journeys: [
        { endpoint: "GET /products", weight: 4 },
        { endpoint: "POST /orders", weight: 1 },
      ],
    },
  },
});

async function show(v: DraftView) {
  mockFetch([{ json: v }]);
  await useDraft.getState().load();
  render(<TrafficMix />);
}

beforeEach(() => useDraft.setState({ view: null, error: null }));
afterEach(cleanup);

test("each_endpoint_shows_its_share_and_what_that_is_per_second", async () => {
  await show(mixed);
  expect(screen.getByText("80%")).toBeTruthy();
  expect(screen.getByText("20%")).toBeTruthy();
  expect(screen.getByText(/About 240 requests a second/)).toBeTruthy();
  expect(screen.getByText(/About 60 requests a second/)).toBeTruthy();
});

test("a_new_weight_is_one_patch_and_a_bad_one_is_not_sent", async () => {
  await show(mixed);
  const sent = mockFetch([{ json: mixed }]);
  const box = screen.getByLabelText("Weight of POST /orders") as HTMLInputElement;
  fireEvent.change(box, { target: { value: "0" } });
  fireEvent.blur(box);
  expect(sent).toHaveLength(0);
  expect(box.value).toBe("1"); // the box shows what is really set
  fireEvent.change(box, { target: { value: "4" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["traffic", "peak", "journeys", 1, "weight"], value: 4 }],
  });
});

test("with_no_journeys_nothing_is_shown", async () => {
  await show(view({ nodes: {}, traffic: { peak: { rps: 10, journeys: [] } } }));
  expect(screen.queryByLabelText("Traffic mix")).toBeNull();
});
