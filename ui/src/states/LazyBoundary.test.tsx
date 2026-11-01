import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { lazy } from "react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { LazyBoundary } from "./LazyBoundary";

afterEach(cleanup);

test("a_failed_chunk_import_shows_studio_was_updated_with_a_reload_button", async () => {
  const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
  const Broken = lazy(() =>
    Promise.reject(new Error("Failed to fetch dynamically imported module")),
  );
  const onReload = vi.fn();
  render(
    <LazyBoundary fallback={<p>loading</p>} onReload={onReload}>
      <Broken />
    </LazyBoundary>,
  );
  expect(screen.getByText("loading")).toBeTruthy();
  expect((await screen.findByRole("alert")).textContent).toMatch(/Studio was updated/);
  fireEvent.click(screen.getByRole("button", { name: "Reload" }));
  expect(onReload).toHaveBeenCalled();
  quiet.mockRestore();
});

test("a_chunk_that_loads_replaces_the_fallback", async () => {
  const Fine = lazy(async () => ({ default: () => <p>the tab</p> }));
  render(
    <LazyBoundary fallback={<p>loading</p>}>
      <Fine />
    </LazyBoundary>,
  );
  expect(await screen.findByText("the tab")).toBeTruthy();
  expect(screen.queryByText("loading")).toBeNull();
});
