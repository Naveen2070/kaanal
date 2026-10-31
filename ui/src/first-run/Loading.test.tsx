import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { DRAFT_STALL_MS, FIRST_RUN_CAP_MS, Loading } from "./Loading";
import { QUOTES } from "./quotes";

beforeEach(() => {
  vi.useFakeTimers();
  useDraft.setState({ view: null, error: null });
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const app = <p>the app</p>;
const step = () => screen.getByRole("status").textContent;
const wait = (ms: number) => act(async () => vi.advanceTimersByTimeAsync(ms));

test("there_are_enough_quotes_and_none_is_empty", () => {
  expect(QUOTES.length).toBeGreaterThanOrEqual(12);
  expect(new Set(QUOTES).size).toBe(QUOTES.length);
  for (const q of QUOTES) expect(q.trim().length).toBeGreaterThan(10);
  // spec 12: the mirage lines lead, the 14 original lines stay
  expect(QUOTES[0]).toBe("It looks like a backend. It answers like a backend. It is not one.");
  expect(QUOTES[1]).toBe("Break the mirage here, before production does.");
  expect(QUOTES).toContain("Every system is fast until the queue forms.");
  expect(QUOTES.length).toBeGreaterThanOrEqual(16);
});

test("shows_a_quote_from_the_list_and_no_percentage", () => {
  render(
    <Loading ready={false} random={() => 0.5}>
      {app}
    </Loading>,
  );
  const shown = QUOTES[Math.floor(0.5 * QUOTES.length)] as string;
  expect(screen.getByText(shown)).toBeTruthy();
  expect(document.body.textContent).not.toContain("%");
  expect(screen.queryByText("the app")).toBeNull();
});

test("the_quote_changes_every_four_seconds_and_never_repeats", async () => {
  render(
    <Loading ready={false} random={() => 0}>
      {app}
    </Loading>,
  );
  let shown = QUOTES[0] as string;
  expect(screen.getByText(shown)).toBeTruthy();
  await wait(3900);
  expect(screen.getByText(shown)).toBeTruthy();
  for (let i = 0; i < 3; i++) {
    await wait(i === 0 ? 200 : 4000);
    expect(screen.queryByText(shown)).toBeNull();
    shown = QUOTES.find((q) => screen.queryByText(q)) as string;
    expect(shown).toBeTruthy();
  }
});

test("the_steps_follow_the_draft_and_the_first_run", async () => {
  const { rerender } = render(<Loading ready={false}>{app}</Loading>);
  expect(step()).toBe("Kaanal Studio is loading. Step 1 of 3 · Starting the simulator");
  rerender(<Loading ready>{app}</Loading>);
  expect(step()).toBe("Kaanal Studio is loading. Step 2 of 3 · Reading your draft");
  act(() => useRun.setState({ status: "running" }));
  expect(step()).toBe("Kaanal Studio is loading. Step 3 of 3 · Running the first simulation");
  act(() => useRun.setState({ status: "done" }));
  // the reveal plays, then the app takes over
  expect(screen.queryByText("the app")).toBeNull();
  await wait(900);
  expect(screen.getByText("the app")).toBeTruthy();
});

test("a_slow_load_keeps_the_loader_until_ready", async () => {
  const { rerender } = render(<Loading ready={false}>{app}</Loading>);
  await wait(9000);
  expect(screen.queryByText("the app")).toBeNull();
  act(() => useRun.setState({ status: "done" }));
  rerender(<Loading ready>{app}</Loading>);
  await wait(900);
  expect(screen.getByText("the app")).toBeTruthy();
});

test("a_draft_with_no_traffic_opens_the_app_instead_of_waiting", async () => {
  render(<Loading ready>{app}</Loading>);
  act(() =>
    useRun.setState({ status: "blocked", lastError: "The draft has no traffic profile to run." }),
  );
  await wait(900);
  expect(screen.getByText("the app")).toBeTruthy();
});

test("a_first_run_that_never_ends_opens_the_app_after_the_cap", async () => {
  useRun.setState({ status: "running" });
  render(<Loading ready>{app}</Loading>);
  await wait(FIRST_RUN_CAP_MS - 100);
  expect(screen.queryByText("the app")).toBeNull();
  await wait(100);
  await wait(900);
  expect(screen.getByText("the app")).toBeTruthy();
});

test("a_draft_that_cannot_load_says_why_and_retry_loads_again", () => {
  const load = vi.fn(async () => {});
  useDraft.setState({ error: "connection refused", load });
  render(<Loading ready={false}>{app}</Loading>);
  expect(screen.getByText("Studio could not start: connection refused")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(load).toHaveBeenCalledTimes(1);
  expect(screen.queryByText("the app")).toBeNull();
});

test("reduced_motion_renders_still_art_and_cuts_to_the_app", () => {
  vi.stubGlobal("matchMedia", (q: string) => ({
    matches: q.includes("reduce"),
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
  const { rerender } = render(<Loading ready={false}>{app}</Loading>);
  expect(document.querySelector(".kaanal-shimmer, .kaanal-dash, .kaanal-fade")).toBeNull();
  act(() => useRun.setState({ status: "done" }));
  rerender(<Loading ready>{app}</Loading>);
  expect(screen.getByText("the app")).toBeTruthy();
});

// no aria-busy: assistive technology may hold back a live region inside a busy subtree
test("the_loader_has_one_status_and_is_not_marked_busy", () => {
  render(<Loading ready={false}>{app}</Loading>);
  expect(screen.getByRole("main").getAttribute("aria-busy")).toBeNull();
  expect(screen.getAllByRole("status")).toHaveLength(1);
});

test("a_server_that_never_answers_is_named_after_ten_seconds_with_a_retry", async () => {
  render(<Loading ready={false}>{app}</Loading>);
  expect(step()).toContain("Step 1 of 3");
  await wait(DRAFT_STALL_MS - 100);
  expect(step()).toContain("Step 1 of 3");
  await wait(200);
  expect(step()).toContain("could not start");
  expect(step()).toContain("has not answered for 10 s"); // read out and shown
  expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
});

test("retry_goes_back_to_step_one_and_one_click_starts_one_load", async () => {
  let release!: () => void;
  const load = vi.fn(() => new Promise<void>((r) => (release = r)));
  useDraft.setState({ error: "connection refused", load });
  render(<Loading ready={false}>{app}</Loading>);
  expect(step()).toContain("connection refused");
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(load).toHaveBeenCalledTimes(1);
  await wait(0);
  expect(step()).toContain("Step 1 of 3"); // something visibly happened
  expect(screen.queryByRole("button", { name: "Retry" })).toBeNull(); // no parallel loads
  // it fails again with the same message: back to the failure, announced again
  useDraft.setState({ error: "connection refused" });
  await act(async () => release());
  expect(step()).toContain("connection refused");
});

test("a_retry_that_hangs_again_is_named_again", async () => {
  useDraft.setState({ error: "connection refused", load: () => new Promise<void>(() => {}) });
  render(<Loading ready={false}>{app}</Loading>);
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await wait(DRAFT_STALL_MS + 100);
  expect(step()).toContain("has not answered for 10 s");
});
