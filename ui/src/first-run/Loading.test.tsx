import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { Loading } from "./Loading";
import { QUOTES } from "./quotes";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const app = <p>the app</p>;

test("there_are_enough_quotes_and_none_is_empty", () => {
  expect(QUOTES.length).toBeGreaterThanOrEqual(12);
  expect(new Set(QUOTES).size).toBe(QUOTES.length);
  for (const q of QUOTES) expect(q.trim().length).toBeGreaterThan(10);
});

test("shows_a_quote_from_the_list_and_no_counter", () => {
  render(
    <Loading ready={false} random={() => 0.5}>
      {app}
    </Loading>,
  );
  const shown = QUOTES[Math.floor(0.5 * QUOTES.length)] as string;
  expect(screen.getByText(shown)).toBeTruthy();
  // nothing on the page but the hidden status text and the quote: no counter, no caption
  expect(document.body.textContent).toBe(`Loading Studio${shown}`);
  expect(screen.queryByText("the app")).toBeNull();
});

test("rotates_the_quote_every_four_seconds", async () => {
  render(
    <Loading ready={false} random={() => 0}>
      {app}
    </Loading>,
  );
  expect(screen.getByText(QUOTES[0] as string)).toBeTruthy();
  await act(async () => vi.advanceTimersByTimeAsync(3900));
  expect(screen.getByText(QUOTES[0] as string)).toBeTruthy();
  await act(async () => vi.advanceTimersByTimeAsync(200));
  expect(screen.getByText(QUOTES[1] as string)).toBeTruthy();
  await act(async () => vi.advanceTimersByTimeAsync(4000));
  expect(screen.getByText(QUOTES[2] as string)).toBeTruthy();
});

test("the_quote_wraps_around_the_end_of_the_list", async () => {
  render(
    <Loading ready={false} random={() => 0.999}>
      {app}
    </Loading>,
  );
  expect(screen.getByText(QUOTES.at(-1) as string)).toBeTruthy();
  await act(async () => vi.advanceTimersByTimeAsync(4100));
  expect(screen.getByText(QUOTES[0] as string)).toBeTruthy();
});

test("stays_at_least_600ms_even_when_ready_at_once", async () => {
  render(<Loading ready>{app}</Loading>);
  expect(screen.queryByText("the app")).toBeNull();
  await act(async () => vi.advanceTimersByTimeAsync(500));
  expect(screen.queryByText("the app")).toBeNull();
  await act(async () => vi.advanceTimersByTimeAsync(200));
  expect(screen.getByText("the app")).toBeTruthy();
});

test("a_slow_load_keeps_the_loader_until_ready_then_shows_the_app_at_once", async () => {
  const { rerender } = render(<Loading ready={false}>{app}</Loading>);
  await act(async () => vi.advanceTimersByTimeAsync(9000));
  expect(screen.queryByText("the app")).toBeNull();
  rerender(<Loading ready>{app}</Loading>);
  expect(screen.getByText("the app")).toBeTruthy();
});

test("reduced_motion_renders_a_static_loader", () => {
  vi.stubGlobal("matchMedia", (q: string) => ({
    matches: q.includes("reduce"),
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
  render(<Loading ready={false}>{app}</Loading>);
  const bars = document.querySelector("[data-motion]");
  expect(bars?.getAttribute("data-motion")).toBe("off");
  expect(document.querySelector(".kaanal-bar")).toBeNull();
  expect(document.querySelector(".kaanal-fade")).toBeNull();
});

test("with_motion_the_bars_animate", () => {
  render(<Loading ready={false}>{app}</Loading>);
  expect(document.querySelector("[data-motion]")?.getAttribute("data-motion")).toBe("on");
  expect(document.querySelectorAll(".kaanal-bar")).toHaveLength(6);
});

test("the_loader_has_an_accessible_busy_status", () => {
  render(<Loading ready={false}>{app}</Loading>);
  expect(screen.getByRole("main").getAttribute("aria-busy")).toBe("true");
  expect(screen.getByRole("status").textContent).toBe("Loading Studio");
});
