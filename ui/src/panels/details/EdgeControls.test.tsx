import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { mockFetch } from "../../test/fetch";
import { EMPTY_FEED, FeedContext } from "../../canvas/feed";
import { EdgeControls } from "./EdgeControls";

function view(over: Partial<DraftView["doc"]> = {}): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: {
      nodes: {
        gw: { type: "gateway" },
        app: { type: "service", replicas: 3 },
        db: { type: "database" },
      },
      edges: [
        { from: "gw", to: "app" },
        { from: "app", to: "db", net: "4ms", bandwidth: 2048 },
      ],
      ...over,
    },
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

async function show(index: number, v = view()) {
  mockFetch([{ json: v }]);
  await useDraft.getState().load();
  render(<EdgeControls index={index} />);
}

beforeEach(() => useDraft.setState({ view: null, error: null }));
afterEach(cleanup);

test("a_wire_shows_its_ends_and_its_current_figures", async () => {
  await show(1);
  expect(screen.getByText("app calls db over this wire.")).toBeTruthy();
  expect((screen.getByLabelText("Network latency") as HTMLInputElement).value).toBe("4");
  expect((screen.getByLabelText("Bandwidth") as HTMLInputElement).value).toBe("2");
  expect((screen.getByLabelText("Calls per request") as HTMLInputElement).value).toBe("1");
  expect((screen.getByLabelText("Share of requests (%)") as HTMLInputElement).value).toBe("100");
});

test("a_committed_latency_is_one_patch_at_the_wires_place", async () => {
  await show(1);
  const sent = mockFetch([{ json: view() }]);
  const box = screen.getByLabelText("Network latency");
  fireEvent.change(box, { target: { value: "12" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["edges", 1, "net"], value: "12ms" }],
  });
});

test("bandwidth_is_typed_in_the_unit_picked_and_written_in_bytes", async () => {
  await show(1); // 2048 bytes a second shows as 2 KB/s
  const unit = screen.getByLabelText("Bandwidth unit") as HTMLSelectElement;
  expect(unit.value).toBe("KB/s");
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(unit, { target: { value: "MB/s" } });
  const box = screen.getByLabelText("Bandwidth") as HTMLInputElement;
  expect(Number(box.value)).toBeCloseTo(2 / 1024, 6); // the same amount, in the new unit
  fireEvent.change(box, { target: { value: "120" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["edges", 1, "bandwidth"], value: 120 * 1024 * 1024 }],
  });
});

test("a_rounded_figure_in_a_bigger_unit_is_never_written_back_on_blur", async () => {
  // 125 000 KB/s in GB/s is 0.119209...: the box rounds it, and leaving the box must not change the file
  await show(0, view({ edges: [{ from: "app", to: "db", bandwidth: 125_000 * 1024 }] }));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByLabelText("Bandwidth unit"), { target: { value: "GB/s" } });
  const box = screen.getByLabelText("Bandwidth") as HTMLInputElement;
  expect(box.value).toBe("0.119209");
  fireEvent.blur(box);
  fireEvent.keyDown(box, { key: "Enter" });
  expect(sent).toHaveLength(0);
});

test("latency_is_typed_in_seconds_and_written_in_milliseconds", async () => {
  await show(1);
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByLabelText("Network latency unit"), { target: { value: "s" } });
  const box = screen.getByLabelText("Network latency");
  fireEvent.change(box, { target: { value: "2" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["edges", 1, "net"], value: "2000ms" }],
  });
});

test("a_value_the_loader_would_refuse_is_never_sent", async () => {
  await show(1);
  const sent = mockFetch([{ json: view() }]);
  const share = screen.getByLabelText("Share of requests (%)") as HTMLInputElement;
  fireEvent.change(share, { target: { value: "150" } });
  fireEvent.blur(share);
  const calls = screen.getByLabelText("Calls per request") as HTMLInputElement;
  fireEvent.change(calls, { target: { value: "0" } });
  fireEvent.blur(calls);
  expect(sent).toHaveLength(0);
  expect(share.value).toBe("100"); // the box shows what is really set
});

test("a_wire_out_of_a_gateway_has_no_share_and_says_why", async () => {
  await show(0);
  expect(screen.queryByLabelText("Share of requests (%)")).toBeNull();
  expect(screen.getByText(/out of a gateway/)).toBeTruthy();
});

test("balance_is_offered_only_into_a_component_with_copies_and_the_limit_can_be_removed", async () => {
  await show(0); // into app, which has 3 copies
  expect(screen.getByLabelText("Balance")).toBeTruthy();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByLabelText("Balance"), { target: { value: "random" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["edges", 0, "balance"], value: "random" }],
  });
  cleanup();

  await show(1); // into db, one copy
  expect(screen.queryByLabelText("Balance")).toBeNull();
  const more = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Remove the bandwidth limit" }));
  await vi.waitFor(() => expect(more).toHaveLength(1));
  expect(more[0]?.body).toEqual({
    patch: [{ op: "remove", path: ["edges", 1, "bandwidth"] }],
  });
});

test("a_latency_spread_is_not_editable_here", async () => {
  await show(
    1,
    view({
      edges: [
        { from: "app", to: "db" },
        { from: "app", to: "db", net: { p50: "4ms", p99: "9ms" } },
      ],
    }),
  );
  expect(screen.queryByLabelText("Network latency")).toBeNull();
  expect(screen.getByText(/spread of values/)).toBeTruthy();
});

test("a_full_wire_says_what_bandwidth_the_load_needs", async () => {
  mockFetch([{ json: view() }]);
  await useDraft.getState().load();
  const link = (saturated: boolean, offered: number) => ({
    from: "app",
    to: "db",
    transfers: 10,
    mean_wait_ms: 5,
    offered_load: offered,
    utilization: 1,
    saturated,
    saturates_at_req_s: 40,
    series: { window_s: 1, latency: [] },
  });
  const feed = (l: ReturnType<typeof link>) => ({ ...EMPTY_FEED, link: () => l });
  render(
    <FeedContext.Provider value={feed(link(true, 3))}>
      <EdgeControls index={1} />
    </FeedContext.Provider>,
  );
  // 2 KB/s asked for 3 times over: at least 6 KB/s
  expect(screen.getByText("Full at this load: it needs at least 6 KB/s.")).toBeTruthy();
  cleanup();
  render(
    <FeedContext.Provider value={feed(link(false, 0.5))}>
      <EdgeControls index={1} />
    </FeedContext.Provider>,
  );
  expect(screen.queryByText(/Full at this load/)).toBeNull();
});

test("a_wire_written_with_a_unit_shows_the_same_number_as_its_bytes", async () => {
  const withBandwidth = (bandwidth: unknown) =>
    view({
      edges: [
        { from: "gw", to: "app" },
        { from: "app", to: "db", bandwidth },
      ],
    });
  await show(1, withBandwidth("250KB"));
  expect((screen.getByLabelText("Bandwidth") as HTMLInputElement).value).toBe("250");
  expect((screen.getByLabelText("Bandwidth unit") as HTMLSelectElement).value).toBe("KB/s");
  cleanup();
  await show(1, withBandwidth("2 MB/s"));
  expect((screen.getByLabelText("Bandwidth") as HTMLInputElement).value).toBe("2");
  expect((screen.getByLabelText("Bandwidth unit") as HTMLSelectElement).value).toBe("MB/s");
  cleanup();
  await show(1, withBandwidth("lots")); // not a size: an empty box, never NaN
  expect((screen.getByLabelText("Bandwidth") as HTMLInputElement).value).toBe("");
});

test("request_and_response_size_are_typed_in_a_unit_and_written_in_bytes", async () => {
  await show(1);
  expect((screen.getByLabelText("Request size") as HTMLInputElement).value).toBe("");
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByLabelText("Request size unit"), { target: { value: "KB" } });
  const box = screen.getByLabelText("Request size");
  fireEvent.change(box, { target: { value: "200" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "sends"], value: 200 * 1024 }],
  });
});

test("a_size_that_is_set_can_be_removed_to_go_back_to_passed_on", async () => {
  await show(0, view({ edges: [{ from: "app", to: "db", returns: "5MB" }] }));
  expect((screen.getByLabelText("Response size") as HTMLInputElement).value).toBe("5");
  expect(screen.queryByText("Remove the request size")).toBeNull();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByText("Remove the response size"));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["edges", 0, "returns"] }] });
});

test("secure_connection_writes_a_tls_version_and_none_removes_the_key", async () => {
  await show(1);
  const select = screen.getByLabelText("Secure connection") as HTMLSelectElement;
  expect(select.value).toBe("off");
  expect([...select.options].map((o) => o.value)).toEqual(["off", "1.3", "1.2"]);
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(select, { target: { value: "1.2" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "tls"], value: 1.2 }],
  });
});

test("tls_true_and_1_3_both_read_as_tls_1_3_and_none_removes_the_key", async () => {
  await show(0, view({ edges: [{ from: "app", to: "db", tls: true }] }));
  const select = screen.getByLabelText("Secure connection") as HTMLSelectElement;
  expect(select.value).toBe("1.3");
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(select, { target: { value: "off" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["edges", 0, "tls"] }] });
});

test("calls_per_connection_shows_the_default_as_a_placeholder_and_is_a_whole_number", async () => {
  await show(1);
  const box = screen.getByLabelText("Calls per connection") as HTMLInputElement;
  expect(box.value).toBe("");
  expect(box.placeholder).toBe("1000 (default)");
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(box, { target: { value: "1.5" } });
  fireEvent.blur(box);
  fireEvent.change(box, { target: { value: "0" } });
  fireEvent.blur(box);
  expect(sent).toHaveLength(0); // not whole, then below 1: never sent
  fireEvent.change(box, { target: { value: "50" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "keepalive"], value: 50 }],
  });
});

test("a_keepalive_that_is_set_can_go_back_to_the_default", async () => {
  await show(0, view({ edges: [{ from: "app", to: "db", keepalive: 1 }] }));
  expect((screen.getByLabelText("Calls per connection") as HTMLInputElement).value).toBe("1");
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByText("Back to the default"));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["edges", 0, "keepalive"] }] });
});

test("stream_the_response_has_five_choices_and_default_removes_the_key", async () => {
  await show(1);
  const select = screen.getByLabelText("Stream the response") as HTMLSelectElement;
  expect(select.value).toBe("default");
  expect([...select.options].map((o) => o.value)).toEqual([
    "default",
    "both",
    "response",
    "request",
    "none",
  ]);
  let sent = mockFetch([{ json: view() }]);
  fireEvent.change(select, { target: { value: "none" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "stream"], value: false }],
  });
  cleanup();
  await show(0, view({ edges: [{ from: "app", to: "db", stream: true }] }));
  const set = screen.getByLabelText("Stream the response") as HTMLSelectElement;
  expect(set.value).toBe("both");
  sent = mockFetch([{ json: view() }]);
  fireEvent.change(set, { target: { value: "request" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["edges", 0, "stream"], value: "request" }],
  });
  cleanup();
  await show(0, view({ edges: [{ from: "app", to: "db", stream: "response" }] }));
  const word = screen.getByLabelText("Stream the response") as HTMLSelectElement;
  expect(word.value).toBe("response");
  sent = mockFetch([{ json: view() }]);
  fireEvent.change(word, { target: { value: "default" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["edges", 0, "stream"] }] });
});

test("packet_loss_is_a_percent_written_as_a_probability_and_zero_removes_it", async () => {
  await show(1);
  const box = screen.getByLabelText("Packet loss (%)") as HTMLInputElement;
  expect(box.value).toBe("");
  let sent = mockFetch([{ json: view() }]);
  fireEvent.change(box, { target: { value: "1.5" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "loss"], value: 0.015 }],
  });
  cleanup();
  await show(0, view({ edges: [{ from: "app", to: "db", loss: 0.02 }] }));
  const set = screen.getByLabelText("Packet loss (%)") as HTMLInputElement;
  expect(set.value).toBe("2");
  sent = mockFetch([{ json: view() }]);
  fireEvent.change(set, { target: { value: "0" } });
  fireEvent.blur(set);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["edges", 0, "loss"] }] });
});

test("a_loss_above_100_percent_is_never_sent", async () => {
  await show(1);
  const sent = mockFetch([{ json: view() }]);
  const box = screen.getByLabelText("Packet loss (%)");
  fireEvent.change(box, { target: { value: "150" } });
  fireEvent.blur(box);
  expect(sent).toHaveLength(0);
});

test("compress_is_a_percent_kept_and_can_be_stopped", async () => {
  await show(1);
  let sent = mockFetch([{ json: view() }]);
  const box = screen.getByLabelText("Compress the response (%)");
  fireEvent.change(box, { target: { value: "30" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "compress"], value: 0.3 }],
  });
  cleanup();
  await show(0, view({ edges: [{ from: "app", to: "db", compress: 0.5 }] }));
  expect((screen.getByLabelText("Compress the response (%)") as HTMLInputElement).value).toBe("50");
  sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByText("Stop compressing"));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["edges", 0, "compress"] }] });
});
