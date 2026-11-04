import { fireEvent, render, screen } from "@testing-library/react";
import { expect, vi } from "vite-plus/test";
import type { DraftView, EdgeDoc, NodeDoc } from "../../data/api/types";
import { FeedContext } from "../../canvas/feed";
import { useDraft } from "../../data/store/draft";
import { mockFetch } from "../../test/fetch";
import { nodeSummary } from "../../test/summary";
import { Controls } from "./Controls";

export const app: NodeDoc = { type: "service", concurrency: 12, proc: "98ms", replicas: 2 };
export const edges: EdgeDoc[] = [
  { from: "gw", to: "app" },
  { from: "app", to: "db" },
];

export function view(over: Partial<DraftView> = {}): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: { nodes: { gw: { type: "gateway" }, app, db: { type: "database" } }, edges },
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
    ...over,
  };
}

export const topology = {
  nodes: [
    { name: "gw", type: "gateway" },
    { name: "app", type: "service" },
    { name: "db", type: "database" },
  ],
  edges: [
    { from: "gw", to: "app" },
    { from: "app", to: "db" },
  ],
  routes: [],
  endpoints: [],
};

export async function show(
  summary = nodeSummary(),
  cfg: NodeDoc = app,
  opts: { applied?: boolean; name?: string } = {},
) {
  const name = opts.name ?? "app";
  // the draft, then (when the panel asks) the running system
  mockFetch([
    { json: view({ doc: { nodes: { [name]: cfg }, edges } }) },
    opts.applied === false ? { status: 409, json: { error: "no system" } } : { json: topology },
  ]);
  await useDraft.getState().load();
  const calls = { onDeleted: vi.fn(), onRenamed: vi.fn(), onChaos: vi.fn() };
  render(
    <FeedContext.Provider
      value={{
        node: (n) => (n === name ? summary : undefined),
        nodes: [],
        dimmed: false,
        peakVisits: 0,
      }}
    >
      <Controls name={name} config={cfg} edges={edges} {...calls} />
    </FeedContext.Provider>,
  );
  return calls;
}

export async function change(label: string, value: string) {
  const sent = mockFetch([{ json: view() }]);
  const input = screen.getByLabelText(label);
  fireEvent.change(input, { target: { value } });
  fireEvent.keyDown(input, { key: "Enter" });
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  return sent[0]?.body;
}
