import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { FeedRow } from "../../data/api/logFeed";
import { useAdmin } from "../../data/store/admin";
import { expectNoViolations } from "../../test/axe";

const started = vi.hoisted(() => ({ on: vi.fn(), off: vi.fn() }));
vi.mock("../../data/api/logFeed", () => ({
  startFeed: (cb: (rows: FeedRow[]) => void) => {
    started.on(cb);
    return started.off;
  },
}));

vi.mock("./PayloadEditor.lazy", () => ({
  PayloadEditor: ({ route, onClose }: { route: string; onClose: () => void }) => (
    <dialog open aria-label={`editor for ${route}`}>
      <button type="button" onClick={onClose}>
        close editor
      </button>
    </dialog>
  ),
}));

import { AdminFeed } from "./AdminFeed";

const row = (done: number, over: Partial<FeedRow> = {}): FeedRow => ({
  id: `r-${done}`,
  done,
  ts_ms: Date.UTC(2026, 10, 9, 10, 0, done),
  method: "GET",
  path: `/users/${done}`,
  endpoint: "GET /users/{id}",
  status: 200,
  ms: 3,
  ...over,
});

beforeEach(() => {
  useAdmin.getState().reset();
  started.on.mockClear();
  started.off.mockClear();
});
afterEach(cleanup);

test("it_starts_the_poller_on_mount_and_stops_it_on_unmount", () => {
  const { unmount } = render(<AdminFeed />);
  expect(started.on).toHaveBeenCalledTimes(1);
  unmount();
  expect(started.off).toHaveBeenCalledTimes(1);
});

test("rows_from_the_poller_show_newest_first_and_an_edited_row_says_edited_in_words", async () => {
  render(<AdminFeed />);
  await act(async () => {
    started.on.mock.calls[0]?.[0]([row(1), row(2, { edited: true, status: 503 })]);
  });
  const rows = screen.getAllByRole("row").slice(1); // the first is the header
  expect(rows).toHaveLength(2);
  expect(rows[0]?.textContent).toContain("/users/2");
  expect(rows[0]?.textContent).toContain("edited");
  expect(rows[0]?.textContent).toContain("503");
  expect(rows[1]?.textContent).not.toContain("edited");
});

test("it_says_what_it_waits_for_when_there_is_no_request_yet", () => {
  render(<AdminFeed />);
  expect(screen.getByText(/Point your frontend at the mock/)).toBeTruthy();
});

test("edit_response_is_disabled_for_a_refused_request_and_a_served_one_opens_the_handler", async () => {
  const onEdit = vi.fn();
  render(<AdminFeed onEdit={onEdit} />);
  await act(async () => {
    started.on.mock.calls[0]?.[0]([
      row(1),
      row(2, { endpoint: null, rejected: "unmatched", status: 404 }),
    ]);
  });
  const buttons = screen.getAllByRole("button", { name: /Edit response/ });
  expect(buttons).toHaveLength(2);
  // newest first: the unmatched one is disabled, the served one opens the editor
  expect((buttons[0] as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(buttons[1] as HTMLElement);
  expect(onEdit).toHaveBeenCalledWith(expect.objectContaining({ id: "r-1" }));
});

test("has_no_axe_violations", async () => {
  const { container } = render(<AdminFeed onEdit={() => {}} />);
  await act(async () => {
    started.on.mock.calls[0]?.[0]([row(1)]);
  });
  await expectNoViolations(container);
});

test("without_a_handler_a_served_row_opens_the_editor_on_its_route_and_it_closes", async () => {
  render(<AdminFeed />);
  await act(async () => {
    started.on.mock.calls[0]?.[0]([row(1)]);
  });
  fireEvent.click(screen.getByRole("button", { name: /Edit response/ }));
  expect(await screen.findByRole("dialog", { name: "editor for GET /users/{id}" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "close editor" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("a_head_request_edits_the_route_of_its_get", async () => {
  render(<AdminFeed />);
  await act(async () => {
    started.on.mock.calls[0]?.[0]([row(1, { method: "HEAD", endpoint: "HEAD /users/{id}" })]);
  });
  fireEvent.click(screen.getByRole("button", { name: /Edit response/ }));
  expect(await screen.findByRole("dialog", { name: "editor for GET /users/{id}" })).toBeTruthy();
});
