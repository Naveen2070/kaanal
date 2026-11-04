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

const starterStorage: NodeDoc = {
  type: "storage",
  flavor: "s3",
  first_byte: "25ms",
  throughput: "80MB/s",
};
const PLACEHOLDERS = /These numbers are placeholders, not published figures/;

test("a_storage_node_has_first_byte_throughput_and_flavor_and_no_service_time_or_style", async () => {
  await show(nodeSummary(), { ...starterStorage, concurrency: 8 }, { name: "assets" });
  for (const label of ["Flavor", "Throughput", "Queue limit", "Timeout", "Sends up to"]) {
    expect(screen.queryByLabelText(label), label).toBeTruthy();
  }
  expect(screen.queryByLabelText("Service time")).toBeNull();
  expect(screen.queryByLabelText("Server style")).toBeNull();
  expect(screen.queryByText(/prices its work per operation/)).toBeNull();
  // the starter first byte is a plain duration, so the box is there to edit
  expect((screen.getByLabelText("First byte") as HTMLInputElement).value).toBe("25");
  expect(screen.queryByText(/First byte is a spread of values/)).toBeNull();
  expect(screen.getByText("A label. It sets no numbers.")).toBeTruthy();
  expect((screen.getByLabelText("Flavor") as HTMLSelectElement).value).toBe("s3");
  expect(
    [...(screen.getByLabelText("Flavor") as HTMLSelectElement).options].map((o) => o.value),
  ).toEqual(["s3", "gcs", "blob"]);
});

test("a_first_byte_spread_has_a_p50_and_a_p99_box_that_write_the_pair_together", async () => {
  await show(
    nodeSummary(),
    { ...starterStorage, first_byte: { p50: "20ms", p99: "80ms" } },
    { name: "assets" },
  );
  expect(screen.queryByLabelText("First byte")).toBeNull();
  expect((screen.getByLabelText("First byte p50") as HTMLInputElement).value).toBe("20");
  expect((screen.getByLabelText("First byte p99") as HTMLInputElement).value).toBe("80");
  expect(await change("First byte p99", "120")).toEqual({
    patch: [
      { op: "set", path: ["nodes", "assets", "first_byte"], value: { p50: "20ms", p99: "120ms" } },
    ],
  });
  // a spread is the user's own number, so it is not the placeholder
  expect(screen.queryByText(PLACEHOLDERS)).toBeNull();
});

test("a_first_byte_mean_has_its_own_box_and_another_shape_stays_in_the_file", async () => {
  await show(
    nodeSummary(),
    { ...starterStorage, first_byte: { mean: "20ms" } },
    { name: "assets" },
  );
  expect(screen.queryByLabelText("First byte")).toBeNull();
  expect((screen.getByLabelText("First byte mean") as HTMLInputElement).value).toBe("20");
  expect(await change("First byte mean", "40")).toEqual({
    patch: [{ op: "set", path: ["nodes", "assets", "first_byte"], value: { mean: "40ms" } }],
  });
  cleanup();
  await show(nodeSummary(), { ...starterStorage, first_byte: { p50: "5ms" } }, { name: "assets" });
  expect(screen.queryByLabelText("First byte mean")).toBeNull();
  expect(screen.getByText(/Edit it in kaanal.yaml/)).toBeTruthy();
});

test("first_byte_is_a_time_box_that_writes_first_byte_and_never_proc", async () => {
  await show(nodeSummary(), { type: "storage", first_byte: "20ms" }, { name: "assets" });
  expect((screen.getByLabelText("First byte") as HTMLInputElement).value).toBe("20");
  expect((screen.getByLabelText("First byte unit") as HTMLSelectElement).value).toBe("ms");
  expect(await change("First byte", "30")).toEqual({
    patch: [{ op: "set", path: ["nodes", "assets", "first_byte"], value: "30ms" }],
  });
});

test("throughput_reads_the_files_text_and_writes_whole_bytes_a_second", async () => {
  await show(nodeSummary(), starterStorage, { name: "assets" });
  expect((screen.getByLabelText("Throughput") as HTMLInputElement).value).toBe("80");
  expect((screen.getByLabelText("Throughput unit") as HTMLSelectElement).value).toBe("MB/s");
  expect(await change("Throughput", "100")).toEqual({
    patch: [{ op: "set", path: ["nodes", "assets", "throughput"], value: 104_857_600 }],
  });
  cleanup();
  await show(nodeSummary(), { ...starterStorage, throughput: "250KB/s" }, { name: "assets" });
  expect((screen.getByLabelText("Throughput") as HTMLInputElement).value).toBe("250");
  expect((screen.getByLabelText("Throughput unit") as HTMLSelectElement).value).toBe("KB/s");
});

test("an_untouched_throughput_is_never_written_back_and_remove_clears_it", async () => {
  await show(nodeSummary(), starterStorage, { name: "assets" });
  const sent = mockFetch([{ json: view() }]);
  const box = screen.getByLabelText("Throughput");
  fireEvent.focus(box);
  fireEvent.blur(box);
  fireEvent.keyDown(box, { key: "Enter" });
  expect(sent).toHaveLength(0); // "80MB/s" stays as the file has it
  fireEvent.click(screen.getByRole("button", { name: "Remove the throughput" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "remove", path: ["nodes", "assets", "throughput"] }],
  });
});

test("the_flavor_select_writes_flavor", async () => {
  await show(nodeSummary(), starterStorage, { name: "assets" });
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByLabelText("Flavor"), { target: { value: "gcs" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["nodes", "assets", "flavor"], value: "gcs" }],
  });
});

test("the_placeholder_note_shows_for_the_starter_values_and_goes_when_one_is_edited", async () => {
  await show(nodeSummary(), starterStorage, { name: "assets" });
  expect(screen.getByText(PLACEHOLDERS)).toBeTruthy();
  cleanup();
  await show(nodeSummary(), { ...starterStorage, throughput: "100MB/s" }, { name: "assets" });
  expect(screen.queryByText(PLACEHOLDERS)).toBeNull();
  cleanup();
  await show(nodeSummary(), { ...starterStorage, first_byte: "30ms" }, { name: "assets" });
  expect(screen.queryByText(PLACEHOLDERS)).toBeNull();
});

test("a_rate_limit_on_storage_has_boxes_that_write_the_pair_and_a_remove_button", async () => {
  await show(
    nodeSummary(),
    { ...starterStorage, rate_limit: { rps: 5500, burst: 100 } },
    { name: "assets" },
  );
  expect((screen.getByLabelText("Request limit") as HTMLInputElement).value).toBe("5500");
  expect(await change("Burst", "250")).toEqual({
    patch: [
      { op: "set", path: ["nodes", "assets", "rate_limit"], value: { rps: 5500, burst: 250 } },
    ],
  });
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Remove the request limit" }));
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "remove", path: ["nodes", "assets", "rate_limit"] }],
  });
});

test("a_storage_node_with_no_limit_offers_add_with_the_published_s3_read_rate", async () => {
  await show(nodeSummary(), starterStorage, { name: "assets" });
  expect(screen.queryByLabelText("Request limit")).toBeNull();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Add a request limit" }));
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  expect(sent[0]?.body).toEqual({
    patch: [
      { op: "insert", path: ["nodes", "assets", "rate_limit"], value: { rps: 5500, burst: 5500 } },
    ],
  });
});
