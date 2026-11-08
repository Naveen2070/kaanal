import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { api } from "../../data/api/client";
import type { LiveHello, Tick } from "../../data/api/liveTypes";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useLive } from "../../data/store/live";
import { bucketOf } from "../../data/store/liveMath";
import { useToasts } from "../toast/toasts";
import { expectNoViolations } from "../../test/axe";
import { LiveStrip } from "./LiveStrip";

const pause = vi.hoisted(() => vi.fn(() => Promise.resolve()));
const resume = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("../../data/store/liveRuntime", () => ({ pauseLive: pause, resumeLive: resume }));

const none = { overflow: 0, rate_limit: 0, deadline: 0, breaker: 0, chaos: 0 };
const tick = (t_ms: number, failed = 0): Tick => ({
  t_ms,
  system_ms: t_ms,
  requests: 25,
  failed,
  nodes: [{ arrived: 25, dropped: none, queue: 3, busy_slots: 8, cpu_load: null }],
  wires: [],
  hist: [[bucketOf(40e6), 20]],
  journeys: [],
  status: { rate: 100, paused: false, overloaded: false, in_flight: 0 },
});
const hello = (over: Partial<LiveHello> = {}): LiveHello => ({
  session: 1,
  profile: "peak",
  note: null,
  rate: 100,
  paused: false,
  overloaded: false,
  in_flight: 0,
  submitted: 0,
  clients: 1,
  system_ms: 0,
  ...over,
});

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  useDraft.setState({
    view: {
      doc: { nodes: { app: { type: "service", concurrency: 8 } }, edges: [] },
    } as unknown as DraftView,
  });
  useLive.getState().reset();
  useLive.setState({ status: "running", hello: hello() });
  pause.mockClear();
  resume.mockClear();
  useToasts.setState({ toasts: [] });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

test("the_strip_shows_what_arrives_what_drops_the_percentiles_and_the_busiest", () => {
  useLive.setState({ ticks: [250, 500, 750, 1000].map((t) => tick(t, 2)) });
  render(<LiveStrip />);
  const text = document.body.textContent ?? "";
  expect(text).toContain("100 / s"); // 25 a tick, four ticks a second
  expect(screen.getByText("Dropped this second").nextSibling?.textContent).toBe("8");
  expect(text).toContain("app 100%, 3 waiting");
  expect(text).toContain("Running at real speed");
});

test("pause_and_resume_follow_the_state", () => {
  render(<LiveStrip />);
  fireEvent.click(screen.getByRole("button", { name: "Pause" }));
  expect(pause).toHaveBeenCalledTimes(1);
  act(() => useLive.setState({ status: "paused" }));
  fireEvent.click(screen.getByRole("button", { name: "Resume" }));
  expect(resume).toHaveBeenCalledTimes(1);
  act(() => useLive.setState({ status: "connecting" }));
  expect((screen.getByRole("button", { name: "Pause" }) as HTMLButtonElement).disabled).toBe(true);
});

test("a_typed_rate_is_sent_on_enter_and_a_refusal_is_a_notice", async () => {
  const send = vi
    .spyOn(api, "liveRate")
    .mockResolvedValueOnce({})
    .mockRejectedValueOnce(new Error("too high"));
  render(<LiveStrip />);
  const box = screen.getByRole("textbox", { name: /Offered/ }) as HTMLInputElement;
  fireEvent.change(box, { target: { value: "250" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(send).toHaveBeenCalledWith(250));
  fireEvent.change(box, { target: { value: "999999" } });
  fireEvent.blur(box);
  await vi.waitFor(() =>
    expect(useToasts.getState().toasts.map((t) => t.text)).toContain("too high"),
  );
  // a box left empty or not a number sends nothing
  send.mockClear();
  fireEvent.change(box, { target: { value: "abc" } });
  fireEvent.blur(box);
  expect(send).not.toHaveBeenCalled();
});

test("a_note_and_an_overload_say_why_and_the_polite_sentence_updates_every_few_seconds", async () => {
  useLive.setState({ hello: hello({ note: "the file has no traffic profile", overloaded: true }) });
  render(<LiveStrip />);
  expect(screen.getByText("the file has no traffic profile")).toBeTruthy();
  expect(screen.getByText(/cannot keep up/)).toBeTruthy();
  const live = document.querySelector("output[aria-live=polite]");
  const before = live?.textContent;
  act(() => useLive.setState({ ticks: [250, 500].map((t) => tick(t, 4)) }));
  expect(live?.textContent).toBe(before);
  await act(async () => {
    vi.advanceTimersByTime(3100);
  });
  expect(live?.textContent).toContain("dropped a second");
});

test("the_strip_has_no_accessibility_violations", async () => {
  useLive.setState({ ticks: [250, 500].map((t) => tick(t)) });
  const { container } = render(<LiveStrip />);
  await expectNoViolations(container);
});

// review finding 8
test("the_rate_box_and_the_overload_warning_follow_the_latest_tick_not_the_hello", () => {
  const t = tick(250);
  useLive.setState({
    ticks: [{ ...t, status: { rate: 500, paused: false, overloaded: true, in_flight: 9 } }],
  });
  render(<LiveStrip />);
  expect((screen.getByRole("textbox", { name: /Offered/ }) as HTMLInputElement).value).toBe("500");
  expect(screen.getByText(/cannot keep up/)).toBeTruthy();
});
