import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { mockFetch } from "../../test/fetch";
import { view } from "../../panels/details/controlsHarness";
import { useDraft } from "./draft";
import type { useRouteOwners as UseRouteOwners } from "./owners";

const { useRouteOwners } = await vi.importActual<{ useRouteOwners: typeof UseRouteOwners }>(
  "./owners",
);

beforeEach(() => {
  vi.useFakeTimers();
  useDraft.setState({ view: view(), error: null, restarted: false });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

test("the_owners_come_from_the_validate_answer_a_moment_after_the_draft_settles", async () => {
  const sent = mockFetch([
    { json: { ok: true, diagnostics: [], warnings: [], flagged: [], owners: { "GET /a": "app" } } },
  ]);
  const { result } = renderHook(() => useRouteOwners());
  expect(result.current).toEqual({});
  expect(sent).toHaveLength(0);
  await act(() => vi.advanceTimersByTimeAsync(500));
  expect(sent[0]?.url).toContain("/draft/validate");
  expect(result.current).toEqual({ "GET /a": "app" });
});

test("a_draft_that_does_not_build_has_no_owners_and_a_disabled_hook_asks_nothing", async () => {
  const sent = mockFetch([
    { json: { ok: false, diagnostics: [], warnings: [], flagged: [], owners: {} } },
  ]);
  const off = renderHook(() => useRouteOwners(false));
  await act(() => vi.advanceTimersByTimeAsync(500));
  expect(sent).toHaveLength(0);
  expect(off.result.current).toEqual({});
  const on = renderHook(() => useRouteOwners());
  await act(() => vi.advanceTimersByTimeAsync(500));
  expect(on.result.current).toEqual({});
});
