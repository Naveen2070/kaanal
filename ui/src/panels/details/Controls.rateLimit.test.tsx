import { cleanup, fireEvent, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { NodeDoc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useToasts } from "../../shell/toast/toasts";
import { mockFetch } from "../../test/fetch";
import { nodeSummary } from "../../test/summary";
import { change, show, view } from "./controlsHarness";

beforeEach(() => {
  useDraft.setState({ view: null, error: null, restarted: false });
  useToasts.setState({ toasts: [] });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** One of every kind the config accepts a `rate_limit` on, each as a node a user could have. */
const KINDS: [string, NodeDoc][] = [
  ["gateway", { type: "gateway" }],
  ["service", { type: "service", proc: "10ms" }],
  ["monolith", { type: "monolith", proc: "10ms" }],
  ["database", { type: "database" }],
  ["cache", { type: "cache" }],
  ["external-api", { type: "external-api", proc: "50ms" }],
  ["auth", { type: "auth", proc: "5ms" }],
  ["load-balancer", { type: "load-balancer" }],
  ["storage", { type: "storage", flavor: "s3", first_byte: "25ms" }],
];

test.each(KINDS)(
  "%s_offers_a_request_limit_that_starts_as_a_labelled_placeholder",
  async (kind, cfg) => {
    await show(nodeSummary(), cfg, { name: "n" });
    expect(screen.queryByLabelText("Request limit"), kind).toBeNull();
    const sent = mockFetch([{ json: view() }]);
    fireEvent.click(screen.getByRole("button", { name: "Add a request limit" }));
    await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
    // a store starts at S3's published read rate; every other kind at a placeholder
    const start = kind === "storage" ? 5500 : 1000;
    expect(sent[0]?.body).toEqual({
      patch: [
        { op: "insert", path: ["nodes", "n", "rate_limit"], value: { rps: start, burst: start } },
      ],
    });
    expect(
      screen.getByText(
        kind === "storage" ? /the figure S3 publishes/ : /a placeholder and not a published figure/,
      ),
    ).toBeTruthy();
  },
);

test.each(KINDS)("%s_with_a_limit_edits_the_pair_and_removes_it", async (kind, cfg) => {
  await show(nodeSummary(), { ...cfg, rate_limit: { rps: 200, burst: 50 } }, { name: "n" });
  expect((screen.getByLabelText("Request limit") as HTMLInputElement).value).toBe("200");
  expect((screen.getByLabelText("Burst") as HTMLInputElement).value).toBe("50");
  expect(await change("Request limit", "300"), kind).toEqual({
    patch: [{ op: "set", path: ["nodes", "n", "rate_limit"], value: { rps: 300, burst: 50 } }],
  });
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Remove the request limit" }));
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["nodes", "n", "rate_limit"] }] });
});

test("the_hint_names_the_status_a_turned_away_request_gets_503_at_a_store_and_429_elsewhere", async () => {
  await show(
    nodeSummary(),
    { type: "service", proc: "10ms", rate_limit: { rps: 100, burst: 10 } },
    { name: "n" },
  );
  expect(screen.getByText(/turned away with a 429/)).toBeTruthy();
  cleanup();
  await show(
    nodeSummary(),
    { type: "storage", first_byte: "25ms", rate_limit: { rps: 100, burst: 10 } },
    { name: "n" },
  );
  expect(screen.getByText(/turned away with a 503/)).toBeTruthy();
});

test("a_limit_written_in_a_form_the_boxes_cannot_show_says_so_and_offers_no_add", async () => {
  await show(
    nodeSummary(),
    { type: "service", proc: "10ms", rate_limit: { rps: "fast" } },
    { name: "n" },
  );
  expect(screen.getByText(/A request limit is set in a form the boxes cannot show/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Add a request limit" })).toBeNull();
});

test("a_burst_below_one_or_a_limit_above_the_loaders_ceiling_is_never_sent", async () => {
  await show(
    nodeSummary(),
    { type: "service", proc: "10ms", rate_limit: { rps: 200, burst: 50 } },
    { name: "n" },
  );
  const sent = mockFetch([{ json: view() }]);
  const burst = screen.getByLabelText("Burst");
  fireEvent.change(burst, { target: { value: "0" } });
  fireEvent.blur(burst);
  const rps = screen.getByLabelText("Request limit");
  fireEvent.change(rps, { target: { value: "1e12" } });
  fireEvent.blur(rps);
  expect(sent).toHaveLength(0);
});
