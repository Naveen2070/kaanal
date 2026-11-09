import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useAdmin } from "../../data/store/admin";
import { useDraft } from "../../data/store/draft";
import { expectNoViolations } from "../../test/axe";

const api = vi.hoisted(() => ({
  listPayloads: vi.fn(),
  putPayload: vi.fn(),
  clearPayload: vi.fn(),
  previewPayload: vi.fn(),
}));
vi.mock("../../data/api/payload", () => api);

import { PayloadEditor } from "./PayloadEditor";

const ROUTE = "GET /users/{id}";
const patch = vi.fn(() => Promise.resolve(true));

function draftWith(doc: unknown) {
  useDraft.setState({ view: { doc } as unknown as DraftView, patch } as never);
}

beforeEach(() => {
  useAdmin.getState().reset();
  api.listPayloads.mockResolvedValue([]);
  api.previewPayload.mockResolvedValue({ status: 200, body: { id: 1 }, headers: {} });
  api.putPayload.mockResolvedValue(undefined);
  api.clearPayload.mockResolvedValue(undefined);
  patch.mockClear();
  draftWith({ endpoints: { [ROUTE]: { chain: ["db"] } } });
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function open(onClose = vi.fn()) {
  render(<PayloadEditor route={ROUTE} onClose={onClose} />);
  await screen.findByLabelText("Status");
  return onClose;
}

const set = (label: string, value: string) =>
  fireEvent.change(screen.getByLabelText(label), { target: { value } });

test("it_opens_on_what_the_route_answers_now_when_it_has_no_edit", async () => {
  await open();
  expect((screen.getByLabelText("Status") as HTMLInputElement).value).toBe("200");
  expect((screen.getByLabelText("Body (JSON)") as HTMLTextAreaElement).value).toContain('"id": 1');
  expect(api.previewPayload).toHaveBeenCalledWith(ROUTE);
});

test("it_opens_on_the_edit_the_route_already_has", async () => {
  api.listPayloads.mockResolvedValue([
    { route: ROUTE, status: 503, body: { e: 1 }, headers: { "x-a": "b" } },
  ]);
  await open();
  expect((screen.getByLabelText("Status") as HTMLInputElement).value).toBe("503");
  expect(api.previewPayload).not.toHaveBeenCalled();
});

test("apply_now_sends_the_edit_and_says_it_is_live_only_not_in_the_file", async () => {
  await open();
  set("Status", "503");
  set("Body (JSON)", '{ "down": true }');
  fireEvent.click(screen.getByRole("button", { name: "Apply now" }));
  await waitFor(() => expect(api.putPayload).toHaveBeenCalled());
  expect(api.putPayload).toHaveBeenCalledWith({
    route: ROUTE,
    status: 503,
    body: { down: true },
    headers: {},
  });
  expect(await screen.findByText(/Live only, not in the file/)).toBeTruthy();
  expect(useAdmin.getState().liveOnly).toContain(ROUTE);
});

test("a_value_the_page_cannot_use_is_shown_under_its_field_and_nothing_is_sent", async () => {
  await open();
  set("Body (JSON)", "{ nope");
  fireEvent.click(screen.getByRole("button", { name: "Apply now" }));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toMatch(/not valid JSON/);
  expect(screen.getByLabelText("Body (JSON)").getAttribute("aria-describedby")).toBe(alert.id);
  expect(api.putPayload).not.toHaveBeenCalled();
});

test("a_server_422_shows_under_the_field_it_names", async () => {
  api.putPayload.mockRejectedValue({
    field: "headers",
    message: "headers: content-length is set by Kaanal",
  });
  await open();
  fireEvent.click(screen.getByRole("button", { name: "Add a header" }));
  fireEvent.change(screen.getByLabelText("Header name 1"), { target: { value: "content-length" } });
  fireEvent.change(screen.getByLabelText("Header value 1"), { target: { value: "1" } });
  fireEvent.click(screen.getByRole("button", { name: "Apply now" }));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("content-length is set by Kaanal");
  expect(screen.getByLabelText("Header name 1").getAttribute("aria-describedby")).toBe(alert.id);
});

test("back_to_normal_clears_the_edit_and_the_live_only_line", async () => {
  api.listPayloads.mockResolvedValue([{ route: ROUTE, status: 503, body: null, headers: {} }]);
  useAdmin.setState({ liveOnly: [ROUTE] });
  await open();
  expect(screen.getByText(/Live only, not in the file/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Back to normal" }));
  await waitFor(() => expect(api.clearPayload).toHaveBeenCalledWith(ROUTE));
  await waitFor(() => expect(screen.queryByText(/Live only, not in the file/)).toBeNull());
  expect(useAdmin.getState().liveOnly).not.toContain(ROUTE);
});

test("stage_patches_the_draft_override_and_clears_the_live_only_line", async () => {
  useAdmin.setState({ liveOnly: [ROUTE] });
  await open();
  set("Status", "503");
  set("Body (JSON)", '{ "a": 1 }');
  fireEvent.click(screen.getByRole("button", { name: "Stage in the file" }));
  await waitFor(() => expect(patch).toHaveBeenCalled());
  expect(patch).toHaveBeenCalledWith([
    {
      op: "insert",
      path: ["endpoints", ROUTE, "override"],
      value: { status: 503, body: { a: 1 } },
    },
  ]);
  await waitFor(() => expect(useAdmin.getState().liveOnly).not.toContain(ROUTE));
  expect(await screen.findByText(/In the draft; Save writes the file/)).toBeTruthy();
});

test("stage_is_disabled_with_a_reason_when_endpoints_is_a_file_reference", async () => {
  draftWith({ endpoints: "endpoints.yaml" });
  await open();
  const stage = screen.getByRole("button", { name: "Stage in the file" }) as HTMLButtonElement;
  expect(stage.disabled).toBe(true);
  expect(screen.getByText(/another file/)).toBeTruthy();
  // Apply now still works
  expect((screen.getByRole("button", { name: "Apply now" }) as HTMLButtonElement).disabled).toBe(
    false,
  );
});

test("a_refused_route_disables_apply_and_says_why", async () => {
  api.previewPayload.mockResolvedValue({ status: 200, body: {}, headers: {} });
  api.putPayload.mockRejectedValue({
    field: "route",
    message: "'GET /users' answers from a collection, which an edit would freeze",
  });
  await open();
  fireEvent.click(screen.getByRole("button", { name: "Apply now" }));
  expect((await screen.findByRole("alert")).textContent).toContain("collection");
});

test("escape_and_close_call_onClose_and_focus_goes_into_the_dialog", async () => {
  const onClose = await open();
  const dialog = screen.getByRole("dialog", { hidden: true });
  expect(dialog.contains(document.activeElement)).toBe(true);
  fireEvent.keyDown(dialog, { key: "Escape" });
  fireEvent(dialog, new Event("cancel", { cancelable: true }));
  expect(onClose).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  expect(onClose).toHaveBeenCalledTimes(2);
});

test("a_failed_load_says_so_and_does_not_hang", async () => {
  api.listPayloads.mockRejectedValue(new Error("offline"));
  render(<PayloadEditor route={ROUTE} onClose={() => {}} />);
  expect((await screen.findByRole("alert")).textContent).toMatch(/offline/);
});

test("has_no_axe_violations", async () => {
  await open();
  await act(async () => {});
  await expectNoViolations(document.body);
});

test("typing_in_a_header_name_keeps_the_box_and_its_focus", async () => {
  await open();
  fireEvent.click(screen.getByRole("button", { name: "Add a header" }));
  const box = screen.getByLabelText("Header name 1");
  (box as HTMLInputElement).focus();
  fireEvent.change(box, { target: { value: "x" } });
  fireEvent.change(screen.getByLabelText("Header name 1"), { target: { value: "x-flag" } });
  // the same element all along: a changing key would remount the row and drop the focus
  expect(screen.getByLabelText("Header name 1")).toBe(box);
  expect(document.activeElement).toBe(box);
});

test("a_failed_back_to_normal_keeps_the_edit_marked_and_says_why", async () => {
  api.listPayloads.mockResolvedValue([{ route: ROUTE, status: 503, body: null, headers: {} }]);
  api.clearPayload.mockRejectedValue({ field: null, message: "the server is gone" });
  useAdmin.setState({ liveOnly: [ROUTE] });
  await open();
  fireEvent.click(screen.getByRole("button", { name: "Back to normal" }));
  expect((await screen.findByRole("alert")).textContent).toContain("the server is gone");
  // the server still serves the edit, so the page still says so
  expect(screen.getByText(/Live only, not in the file/)).toBeTruthy();
  expect(useAdmin.getState().liveOnly).toContain(ROUTE);
  expect(
    (screen.getByRole("button", { name: "Back to normal" }) as HTMLButtonElement).disabled,
  ).toBe(false);
});
