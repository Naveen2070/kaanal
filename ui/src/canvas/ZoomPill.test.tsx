import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { expectNoViolations } from "../test/axe";
import { canvasBridge } from "./bridge";
import { useZoom, zoomPercent } from "./zoom";
import { ZoomPill } from "./ZoomPill";

beforeEach(() => useZoom.setState({ zoom: 1 }));
afterEach(() => {
  cleanup();
  canvasBridge.zoomIn = () => {};
  canvasBridge.zoomOut = () => {};
  canvasBridge.fit = () => {};
  vi.unstubAllGlobals();
});

test("the_zoom_reads_as_a_percent", () => {
  expect([zoomPercent(1), zoomPercent(0.874), zoomPercent(1.46), zoomPercent(NaN)]).toEqual([
    "100%",
    "87%",
    "146%",
    "100%",
  ]);
  render(<ZoomPill />);
  act(() => useZoom.setState({ zoom: 1.46 }));
  expect(screen.getByRole("status", { name: "Zoom" }).textContent).toBe("146%");
});

test("the_buttons_ask_the_canvas_to_zoom_and_fit", () => {
  canvasBridge.zoomIn = vi.fn();
  canvasBridge.zoomOut = vi.fn();
  canvasBridge.fit = vi.fn();
  render(<ZoomPill />);
  fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
  fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
  fireEvent.click(screen.getByRole("button", { name: "Fit the view" }));
  expect(canvasBridge.zoomIn).toHaveBeenCalledTimes(1);
  expect(canvasBridge.zoomOut).toHaveBeenCalledTimes(1);
  expect(canvasBridge.fit).toHaveBeenCalledTimes(1);
});

test("full_screen_asks_for_the_whole_page_and_says_which_way_it_will_go", async () => {
  const enter = vi.fn(async () => {});
  const leave = vi.fn(async () => {});
  Object.defineProperty(document.documentElement, "requestFullscreen", {
    value: enter,
    configurable: true,
  });
  Object.defineProperty(document, "exitFullscreen", { value: leave, configurable: true });
  let current: Element | null = null;
  Object.defineProperty(document, "fullscreenElement", { get: () => current, configurable: true });
  render(<ZoomPill />);
  fireEvent.click(screen.getByRole("button", { name: "Full screen" }));
  expect(enter).toHaveBeenCalledTimes(1);
  current = document.documentElement;
  await act(async () => void document.dispatchEvent(new Event("fullscreenchange")));
  const back = screen.getByRole("button", { name: "Leave full screen" });
  expect(back.getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(back);
  expect(leave).toHaveBeenCalledTimes(1);
});

test("a_browser_that_refuses_full_screen_does_not_break_the_button", () => {
  Object.defineProperty(document.documentElement, "requestFullscreen", {
    value: () => Promise.reject(new Error("denied")),
    configurable: true,
  });
  Object.defineProperty(document, "fullscreenElement", { get: () => null, configurable: true });
  render(<ZoomPill />);
  expect(() => fireEvent.click(screen.getByRole("button", { name: "Full screen" }))).not.toThrow();
});

test("the_pill_has_no_axe_violations", async () => {
  render(<ZoomPill />);
  await expectNoViolations(document.body);
});
