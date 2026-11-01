import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { history } from "../store/history";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { expectNoViolations } from "../test/axe";
import { startNotifications } from "./bind";
import { ToastHost } from "./ToastHost";
import { AUTO_DISMISS_MS, notify, useToasts } from "./toasts";

beforeEach(() => {
  useToasts.setState({ toasts: [] });
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
  useDraft.setState({ view: null, error: null });
  history.clear();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const texts = () => useToasts.getState().toasts.map((t) => t.text);

test("a_message_is_a_closeable_notice", () => {
  render(<ToastHost />);
  act(() => notify.error("The draft has no traffic profile to run.", "run"));
  expect(screen.getByRole("alert").textContent).toContain("The draft has no traffic profile");
  fireEvent.click(screen.getByRole("button", { name: "Dismiss this notice" }));
  expect(screen.queryByRole("alert")).toBeNull();
});

test("errors_and_warnings_are_alerts_and_the_rest_are_status", () => {
  render(<ToastHost />);
  act(() => {
    notify.error("e");
    notify.warn("w");
    notify.info("i");
    notify.success("s");
  });
  expect(screen.getAllByRole("alert").map((a) => a.textContent)).toEqual([
    expect.stringContaining("e"),
    expect.stringContaining("w"),
  ]);
  expect(screen.getAllByRole("status")).toHaveLength(2);
});

test("the_same_words_are_never_up_twice_and_a_newer_message_from_a_source_replaces_its_older", () => {
  notify.error("same", "a");
  notify.error("same", "b");
  expect(texts()).toEqual(["same"]);
  notify.error("first", "run");
  notify.error("second", "run");
  expect(texts()).toEqual(["same", "second"]);
});

test("at_most_five_stay_and_the_oldest_goes", () => {
  for (let i = 0; i < 8; i++) notify.error(`m${i}`);
  expect(texts()).toEqual(["m3", "m4", "m5", "m6", "m7"]);
});

test("information_and_success_close_themselves_errors_and_warnings_do_not", () => {
  vi.useFakeTimers();
  render(<ToastHost />);
  act(() => {
    notify.error("stays");
    notify.warn("stays too");
    notify.info("goes");
    notify.success("also goes");
  });
  act(() => void vi.advanceTimersByTime(AUTO_DISMISS_MS + 100));
  expect(texts()).toEqual(["stays", "stays too"]);
});

test("a_run_that_cannot_give_numbers_is_a_notice_and_its_recovery_closes_it", () => {
  const off = startNotifications();
  useDraft.setState({ view: { doc: { nodes: { a: {} } } } } as never);
  useRun.setState({ status: "blocked", lastError: "The draft has no traffic profile to run." });
  expect(texts()).toEqual(["The draft has no traffic profile to run."]);
  useRun.setState({ status: "done", lastError: null });
  expect(texts()).toEqual([]);
  off();
});

test("a_refused_edit_is_a_notice_and_the_next_success_closes_it", () => {
  const off = startNotifications();
  useDraft.setState({ error: "out of range" });
  expect(texts()).toEqual(["out of range"]);
  useDraft.setState({ error: null });
  expect(texts()).toEqual([]);
  off();
});

test("an_edit_that_cannot_be_undone_says_so_in_a_notice", () => {
  const off = startNotifications();
  history.skip();
  expect(texts()[0]).toMatch(/can't be undone/);
  off();
});

test("notifications_stop_when_unsubscribed", () => {
  startNotifications()();
  useDraft.setState({ error: "ignored" });
  expect(texts()).toEqual([]);
});

test("the_stack_has_no_axe_violations", async () => {
  render(<ToastHost />);
  act(() => {
    notify.error("An error");
    notify.warn("A warning");
    notify.info("Some information");
    notify.success("It worked");
  });
  await expectNoViolations(document.body);
});

test("a_toast_with_an_action_runs_it_and_closes", () => {
  const run = vi.fn();
  notify.info("Wire deleted.", "wire", { label: "Undo", run });
  render(<ToastHost />);
  fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  expect(run).toHaveBeenCalledTimes(1);
  expect(useToasts.getState().toasts).toHaveLength(0);
});

test("a_toast_under_the_pointer_does_not_close_itself", () => {
  vi.useFakeTimers();
  notify.success("Saved.");
  render(<ToastHost />);
  fireEvent.mouseEnter(screen.getByRole("status"));
  act(() => void vi.advanceTimersByTime(AUTO_DISMISS_MS * 2));
  expect(useToasts.getState().toasts).toHaveLength(1);
  fireEvent.mouseLeave(screen.getByRole("status"));
  act(() => void vi.advanceTimersByTime(AUTO_DISMISS_MS + 10));
  expect(useToasts.getState().toasts).toHaveLength(0);
});

test("a_run_error_on_a_blank_draft_is_not_shown_until_something_is_drawn", () => {
  const off = startNotifications();
  const doc = (nodes: Record<string, unknown>) =>
    ({ view: { doc: { nodes } } }) as unknown as Parameters<typeof useDraft.setState>[0];
  useDraft.setState(doc({}));
  useRun.setState({ lastError: "a topology needs a gateway" });
  expect(useToasts.getState().toasts).toHaveLength(0);
  useDraft.setState(doc({ gw: { type: "gateway" } }));
  expect(useToasts.getState().toasts.map((t) => t.text)).toEqual(["a topology needs a gateway"]);
  off();
});
